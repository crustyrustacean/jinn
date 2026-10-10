//! The graph store: one minigraf handle per named graph, held for the
//! server's life, plus the graph tools behind it.
//!
//! All methods are synchronous — the tools call them from
//! [`tokio::task::spawn_blocking`](tokio::task::spawn_blocking). Every method
//! is auto-commit: one `execute` per call, which minigraf already makes an
//! atomic WAL transaction. Explicit `WriteTransaction`s are deliberately not
//! used: they are `!Send` and poison same-thread `execute` while open
//! (`Int001`).
//!
//! # Create-before-use
//!
//! Only [`create_graph`](MinigrafStore::create_graph) and
//! [`instantiate_graph`](MinigrafStore::instantiate_graph) make `.graph`
//! files. Both validate before claiming and claim with `create_new`, so a
//! name collision or a bad script never touches an existing file.
//!
//! # Templates and replay
//!
//! An instantiated or created graph carries provenance facts (`:graph/…` on
//! the `:graph/self` entity). Facts replay **never**; rules replay **always
//! when available**: opening a graph whose provenance names a template into a
//! fresh handle re-executes only that template's `(rules [...])` forms,
//! because rules are memory-only in minigraf 2.0.3. A missing or broken
//! template degrades — the graph still opens, and that graph's tool results
//! carry a one-line warning — never blocks.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use error_stack::Report;
use jiff::Timestamp;
use parking_lot::Mutex;

use crate::errors::MinigrafServerError;
use crate::json_rows;
use crate::naming;
use crate::templates;

/// The entity provenance facts attach to. A single well-known keyword: it is
/// deterministic, so every graph's provenance is queryable with the same
/// pattern.
pub const PROVENANCE_ENTITY: &str = ":graph/self";

/// One minigraf handle per graph name, plus the directories the `.graph`
/// files and the templates live in. Cheap to clone; every clone sees the same
/// handle cache.
#[derive(Clone)]
pub struct MinigrafStore {
    minigraf_dir: PathBuf,
    templates_dir: PathBuf,
    handles: Arc<Mutex<HashMap<String, minigraf::Minigraf>>>,
    /// One-line replay warnings per graph, written when a fresh handle opens
    /// a provenance-carrying graph whose template could not be replayed.
    /// Read by the tool layer to decorate that graph's results.
    warnings: Arc<Mutex<HashMap<String, String>>>,
}

/// Provenance reported by the activate-type operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphProvenance {
    /// The graph name.
    pub name: String,
    /// The template it was instantiated from; `None` for a blank graph.
    pub template: Option<String>,
    /// The session id jinn attributed the activation to.
    pub created_by: String,
}

impl MinigrafStore {
    /// Builds a store over `minigraf_dir` with templates under
    /// `templates_dir`, without touching the filesystem; call
    /// [`ensure_dir`](Self::ensure_dir) to create the graph directory.
    #[must_use]
    pub fn new(minigraf_dir: PathBuf, templates_dir: PathBuf) -> Self {
        Self {
            minigraf_dir,
            templates_dir,
            handles: Arc::new(Mutex::new(HashMap::new())),
            warnings: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// A clone of this store sharing the handle and warning caches but
    /// reading templates from `templates_dir` (test seam for pointing a
    /// contender store at a different templates root).
    #[must_use]
    pub fn over_templates(&self, templates_dir: PathBuf) -> Self {
        Self {
            minigraf_dir: self.minigraf_dir.clone(),
            templates_dir,
            handles: Arc::clone(&self.handles),
            warnings: Arc::clone(&self.warnings),
        }
    }

    /// The templates root this store reads `template.dl` scripts from.
    #[must_use]
    pub fn templates_dir(&self) -> &std::path::Path {
        &self.templates_dir
    }

    /// Creates the minigraf directory if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created.
    pub fn ensure_dir(&self) -> Result<(), Report<MinigrafServerError>> {
        fs::create_dir_all(&self.minigraf_dir).map_err(|e| {
            Report::new(MinigrafServerError::StoreDir(format!(
                "{}: {e}",
                self.minigraf_dir.display()
            )))
        })?;
        Ok(())
    }

    /// Creates `<name>.graph` as a blank graph, stamps its provenance, and
    /// returns it (the activate-type contract: creating is activating).
    ///
    /// The file is claimed with `OpenOptions::create_new` (atomic) before
    /// minigraf opens it, so two racing creates cannot both win and a failed
    /// minigraf open leaves a valid empty file behind rather than a half
    /// written one.
    ///
    /// # Errors
    ///
    /// [`MinigrafServerError::BadName`] for an unsafe name,
    /// [`MinigrafServerError::AlreadyExists`] when the graph exists (never
    /// truncating the existing one), [`MinigrafServerError::Minigraf`] when minigraf
    /// refuses the open, and [`MinigrafServerError::StoreDir`] on filesystem errors.
    pub fn create_graph(
        &self,
        name: &str,
        created_by: &str,
    ) -> Result<GraphProvenance, Report<MinigrafServerError>> {
        let db = self.claim(name)?;
        let template = None;
        let provenance = provenance_facts(created_by, template.as_deref());
        for form in &provenance {
            db.execute(form)
                .map_err(|e| Report::new(MinigrafServerError::Minigraf(e.to_string())))?;
        }
        Ok(GraphProvenance {
            name: name.to_owned(),
            template,
            created_by: created_by.to_owned(),
        })
    }

    /// Instantiates a template into a new `<name>.graph`.
    ///
    /// Ordering is the create-before-use invariant: the template script is
    /// validated against an in-memory minigraf **first** (a bad script never
    /// touches the graph directory), then the file is claimed atomically,
    /// then the seed forms, the rule forms, and the provenance facts execute
    /// against the fresh handle.
    ///
    /// # Errors
    ///
    /// [`MinigrafServerError::UnknownTemplate`] when no such template exists,
    /// [`MinigrafServerError::BadTemplate`] when the script fails to parse or
    /// validate, [`MinigrafServerError::AlreadyExists`] when the graph name is
    /// taken (the file is left untouched), [`MinigrafServerError::BadName`]
    /// for unsafe names, [`MinigrafServerError::Minigraf`] for execution
    /// failures after the claim, and [`MinigrafServerError::StoreDir`] on
    /// filesystem errors.
    pub fn instantiate_graph(
        &self,
        template_name: &str,
        name: &str,
        created_by: &str,
    ) -> Result<GraphProvenance, Report<MinigrafServerError>> {
        let template = templates::load_template(&self.templates_dir, template_name)?;
        // Validate in memory: every form must execute against a throwaway
        // database before any file is claimed.
        let probe = minigraf::Minigraf::in_memory()
            .map_err(|e| Report::new(MinigrafServerError::Minigraf(e.to_string())))?;
        template.script.validate_against(&probe)?;

        let db = self.claim(name)?;
        let provenance = provenance_facts(created_by, Some(template_name));
        let forms = template
            .script
            .seed_forms
            .iter()
            .chain(template.script.rule_forms.iter())
            .chain(provenance.iter());
        for form in forms {
            db.execute(form)
                .map_err(|e| Report::new(MinigrafServerError::Minigraf(e.to_string())))?;
        }
        Ok(GraphProvenance {
            name: name.to_owned(),
            template: Some(template.name),
            created_by: created_by.to_owned(),
        })
    }

    /// Binds an existing graph as a session's active graph and stamps attach
    /// provenance onto it.
    ///
    /// An already-open (cached) graph reuses its handle; a foreign-held file
    /// surfaces minigraf's lock refusal (`STG-025`/`STG-026`) as a clean error.
    ///
    /// # Errors
    ///
    /// [`MinigrafServerError::UnknownGraph`] when no such graph exists (the
    /// remedies are `instantiate_graph` or `attach_graph`'s sibling
    /// `create_graph`), plus the [`existing_handle`](Self::existing_handle)
    /// error surface.
    pub fn attach_graph(
        &self,
        name: &str,
        created_by: &str,
    ) -> Result<GraphProvenance, Report<MinigrafServerError>> {
        let db = self.existing_handle(name)?;
        let template = self.template_of(&db)?;
        let form = format!(
            "(transact [[:graph/self :graph/attached-by \"{created_by}\"] \
             [:graph/self :graph/attached-at \"{}\"]])",
            Timestamp::now()
        );
        db.execute(&form)
            .map_err(|e| Report::new(MinigrafServerError::Minigraf(e.to_string())))?;
        Ok(GraphProvenance {
            name: name.to_owned(),
            template,
            created_by: created_by.to_owned(),
        })
    }

    /// The one-line replay warning for a graph, if its last fresh open could
    /// not replay its template's rules.
    #[must_use]
    pub fn warning_for(&self, name: &str) -> Option<String> {
        self.warnings.lock().get(name).cloned()
    }

    /// Runs one Datalog write command against an existing graph; returns the
    /// transaction id (unix millis — the `:as-of` value for reads after this
    /// write).
    ///
    /// # Errors
    ///
    /// [`MinigrafServerError::UnknownGraph`] when the graph does not exist (the
    /// remedy is `create_graph`/`instantiate_graph`), [`MinigrafServerError::BadName`]
    /// for an unsafe name, [`MinigrafServerError::Minigraf`] for parse or execution
    /// failures.
    pub fn transact(&self, graph: &str, datalog: &str) -> Result<u64, Report<MinigrafServerError>> {
        let db = self.existing_handle(graph)?;
        let result = db
            .execute(datalog)
            .map_err(|e| Report::new(MinigrafServerError::Minigraf(e.to_string())))?;
        match result {
            minigraf::QueryResult::Transacted(tx) | minigraf::QueryResult::Retracted(tx) => Ok(tx),
            // `(rule ...)` and degenerate writes acknowledge with `Ok`.
            minigraf::QueryResult::Ok => Ok(0),
            // A write command never produces rows; render honestly if one
            // ever does rather than guessing.
            other => Err(Report::new(MinigrafServerError::Minigraf(format!(
                "write command produced {other:?}"
            )))),
        }
    }

    /// Runs one Datalog `(query ...)` against an existing graph and renders
    /// the rows as JSON.
    ///
    /// # Errors
    ///
    /// [`MinigrafServerError::UnknownGraph`] when the graph does not exist (the
    /// remedy is `create_graph`/`instantiate_graph`), [`MinigrafServerError::BadName`]
    /// for an unsafe name, [`MinigrafServerError::Minigraf`] for parse or execution
    /// failures.
    pub fn query(&self, graph: &str, datalog: &str) -> Result<String, Report<MinigrafServerError>> {
        let db = self.existing_handle(graph)?;
        let result = db
            .execute(datalog)
            .map_err(|e| Report::new(MinigrafServerError::Minigraf(e.to_string())))?;
        Ok(json_rows::render(&result))
    }

    /// Atomically claims `<name>.graph` and opens a handle on it.
    ///
    /// Validation and template loading happen in the callers, **before** this
    /// runs: nothing here can fail with a template error, so a claimed file is
    /// always a real graph.
    fn claim(&self, name: &str) -> Result<minigraf::Minigraf, Report<MinigrafServerError>> {
        naming::validate(name)?;

        let path = self.graph_file(name);
        // Atomic claim: create_new fails if the file exists — nothing on
        // disk is truncated or replaced, ever.
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    Report::new(MinigrafServerError::AlreadyExists(name.to_owned()))
                } else {
                    Report::new(MinigrafServerError::StoreDir(format!(
                        "{}: {e}",
                        path.display()
                    )))
                }
            })?;

        // Claim won: open the handle. A failure here leaves a valid empty
        // file behind — the name still reads as "created" to a later
        // create_graph refusal, and the file is recoverable by deleting it.
        let db = minigraf::Minigraf::open(&path)
            .map_err(|e| Report::new(MinigrafServerError::Minigraf(e.to_string())))?;
        self.handles.lock().insert(name.to_owned(), db.clone());
        Ok(db)
    }

    /// The template this graph's provenance names, or `None` for a graph
    /// without provenance (legacy or blank).
    fn template_of(
        &self,
        db: &minigraf::Minigraf,
    ) -> Result<Option<String>, Report<MinigrafServerError>> {
        let result = db
            .execute(&format!(
                "(query [:find ?t :where [{PROVENANCE_ENTITY} :graph/instance-of ?t]])"
            ))
            .map_err(|e| Report::new(MinigrafServerError::Minigraf(e.to_string())))?;
        match result {
            minigraf::QueryResult::QueryResults { results, .. } => Ok(results
                .into_iter()
                .next()
                .and_then(|row| row.into_iter().next().and_then(minigraf_value_to_string))),
            _ => Ok(None),
        }
    }

    /// Replays a graph's template rules into a freshly opened handle.
    ///
    /// Only `(rules [...])` forms run — never seed forms: the seed is
    /// instantiate-once, and re-running it would overwrite timestamped facts
    /// and re-fire retracts. Any failure (template missing, unreadable, rule
    /// execution failing) degrades to a per-graph warning; the graph still
    /// opens. Replay is a no-op for a graph whose provenance names no template.
    fn replay_rules(&self, name: &str, db: &minigraf::Minigraf) {
        let template_name = match self.template_of(db) {
            Ok(Some(name)) => name,
            Ok(None) => return,
            Err(report) => {
                self.warnings.lock().insert(
                    name.to_owned(),
                    format!("template provenance unreadable; rules not loaded: {report:?}"),
                );
                return;
            }
        };
        let warn = |why: String| {
            self.warnings.lock().insert(
                name.to_owned(),
                format!("template {template_name} unavailable; rules not loaded ({why})"),
            );
        };
        match templates::load_template(&self.templates_dir, &template_name) {
            Ok(template) => {
                for form in &template.script.rule_forms {
                    if let Err(e) = db.execute(form) {
                        warn(format!("{e}"));
                        return;
                    }
                }
            }
            Err(report) => warn(format!("{report:?}")),
        }
    }

    /// Resolves the handle for a graph that must already exist.
    ///
    /// Cache hit returns the held handle (no replay — rules are already
    /// registered on the held handle); a miss checks the file exists first —
    /// a plain minigraf open would *create* the file, which would silently
    /// violate create-before-use — then replays the graph's template rules
    /// into the fresh handle before it is cached.
    fn existing_handle(
        &self,
        name: &str,
    ) -> Result<minigraf::Minigraf, Report<MinigrafServerError>> {
        naming::validate(name)?;
        {
            let handles = self.handles.lock();
            if let Some(db) = handles.get(name) {
                return Ok(db.clone());
            }
        }
        if !self.graph_file(name).exists() {
            return Err(Report::new(MinigrafServerError::UnknownGraph(
                name.to_owned(),
            )));
        }
        // Second process guard: another jinn instance holding this file is
        // minigraf's own lock refusal (STG-026, bounded retry then refusal) —
        // it surfaces as a Minigraf error, never a hang or corruption.
        let db = minigraf::Minigraf::open(self.graph_file(name))
            .map_err(|e| Report::new(MinigrafServerError::Minigraf(e.to_string())))?;
        self.replay_rules(name, &db);
        self.handles.lock().insert(name.to_owned(), db.clone());
        Ok(db)
    }

    fn graph_file(&self, name: &str) -> PathBuf {
        self.minigraf_dir.join(format!("{name}.graph"))
    }
}

/// The provenance transact for a fresh graph.
fn provenance_facts(created_by: &str, template: Option<&str>) -> Vec<String> {
    let mut parts = vec![
        format!("[{PROVENANCE_ENTITY} :graph/created-by \"{created_by}\"]"),
        format!(
            "[{PROVENANCE_ENTITY} :graph/created-at \"{}\"]",
            Timestamp::now()
        ),
    ];
    if let Some(template) = template {
        parts.push(format!(
            "[{PROVENANCE_ENTITY} :graph/instance-of \"{template}\"]"
        ));
    }
    vec![format!("(transact [{}])", parts.join(" "))]
}

/// Renders a minigraf value as the string provenance facts carry.
fn minigraf_value_to_string(value: minigraf::Value) -> Option<String> {
    match value {
        minigraf::Value::String(s) => Some(s),
        minigraf::Value::Keyword(k) => Some(k),
        minigraf::Value::Boolean(b) => Some(b.to_string()),
        minigraf::Value::Integer(i) => Some(i.to_string()),
        _ => None,
    }
}

/// Shared handle alias: minigraf handles are cheap clones over one inner
/// database; the cache holds the authoritative open handle per graph.
pub type SharedHandle = minigraf::Minigraf;

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, reason = "test assertions")]

    use super::*;
    use tempfile::TempDir;

    fn store() -> (TempDir, MinigrafStore) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let store = MinigrafStore::new(
            dir.path().to_path_buf(),
            PathBuf::from("/nonexistent-templates"),
        );
        store.ensure_dir().expect("ensure_dir");
        (dir, store)
    }

    #[rstest::rstest]
    #[test]
    fn create_then_transact_then_query_roundtrips() {
        // Given a store with a created graph.
        let (_dir, store) = store();
        store.create_graph("t1", "test").expect("create");

        // When transacting a fact and querying it back.
        store
            .transact("t1", "(transact [[:alice :task/name \"write tests\"]])")
            .expect("transact");
        let json = store
            .query("t1", "(query [:find ?n :where [?e :task/name ?n]])")
            .expect("query");

        // Then the fact reads back.
        assert!(json.contains("write tests"), "json: {json}");
    }

    #[rstest::rstest]
    #[test]
    fn create_fails_if_graph_exists_and_leaves_contents_intact() {
        // Given a graph with a fact in it.
        let (_dir, store) = store();
        store.create_graph("dup", "test").expect("create");
        store
            .transact("dup", "(transact [[:alice :task/name \"original\"]])")
            .expect("transact");

        // When creating the same name again.
        let result = store.create_graph("dup", "test");

        // Then it fails naming the collision and the existing file is
        // untouched.
        let err = result.expect_err("second create must fail");
        let rendered = format!("{err:?}");
        assert!(
            rendered.contains("already exists"),
            "expected the collision message in: {rendered}"
        );

        // And the original graph's contents are intact.
        let json = store
            .query("dup", "(query [:find ?n :where [?e :task/name ?n]])")
            .expect("query");
        assert!(json.contains("original"), "json: {json}");
    }

    #[rstest::rstest]
    #[test]
    fn transact_on_unknown_graph_names_create_graph_as_remedy() {
        // Given a store where the graph was never created.
        let (_dir, store) = store();

        // When transacting into the unknown graph.
        let result = store.transact("ghost", "(transact [[:a :b \"c\"]])");

        // Then the error names create_graph as the remedy.
        let err = result.expect_err("must refuse");
        let rendered = format!("{err:?}");
        assert!(
            rendered.contains("create_graph"),
            "expected the create_graph remedy in: {rendered}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn query_on_unknown_graph_is_refused_without_creating_a_file() {
        // Given a store where the graph was never created.
        let (_dir, store) = store();

        // When querying the unknown graph.
        let result = store.query("ghost", "(query [:find ?e :where [?e _ _]])");

        // Then it is refused.
        assert!(result.is_err());
        // And no file was created by the refusal.
        assert!(!store.graph_file("ghost").exists());
    }

    #[rstest::rstest]
    #[test]
    fn repeated_transacts_reuse_one_handle() {
        // Given a graph opened once.
        let (_dir, store) = store();
        store.create_graph("cached", "test").expect("create");
        let handles_before = store.handles.lock().len();

        // When several operations run against it.
        store
            .transact("cached", "(transact [[:a :x \"1\"]])")
            .expect("t1");
        store
            .transact("cached", "(transact [[:b :x \"2\"]])")
            .expect("t2");
        store
            .query("cached", "(query [:find ?e :where [?e _ _]])")
            .expect("q");

        // Then the handle count never grew — one handle per graph per process.
        assert_eq!(store.handles.lock().len(), handles_before);
    }

    #[rstest::rstest]
    #[test]
    fn persistence_survives_handle_drop_and_reopen() {
        // Given a graph written in one store (its handles dropped with it).
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().to_path_buf();
        {
            let store = MinigrafStore::new(path.clone(), PathBuf::from("/nonexistent-templates"));
            store.ensure_dir().expect("ensure_dir");
            store.create_graph("durable", "test").expect("create");
            store
                .transact("durable", "(transact [[:alice :task/name \"survives\"]])")
                .expect("transact");
        }

        // When a fresh store over the same directory opens the same graph.
        let store = MinigrafStore::new(path, PathBuf::from("/nonexistent-templates"));
        let json = store
            .query("durable", "(query [:find ?n :where [?e :task/name ?n]])")
            .expect("query");

        // Then the fact survived the restart.
        assert!(json.contains("survives"), "json: {json}");
    }

    #[rstest::rstest]
    #[test]
    fn second_in_process_open_of_same_file_is_refused_not_corrupted() {
        // Given a file-backed graph created directly through minigraf
        // (bypassing the store's cache).
        let dir = TempDir::new().expect("tempdir");
        let db = minigraf::Minigraf::open(dir.path().join("held.graph")).expect("open");

        // When the store discovers the file on a cache miss.
        let store = MinigrafStore::new(
            dir.path().to_path_buf(),
            PathBuf::from("/nonexistent-templates"),
        );
        let result = store.query("held", "(query [:find ?e :where [?e _ _]])");

        // Then the open is refused with minigraf's already-open code, not a
        // second handle.
        let err = result.expect_err("must refuse");
        let rendered = format!("{err:?}");
        assert!(
            rendered.contains("STG-025"),
            "expected the already-open code in: {rendered}"
        );
        drop(db);
    }

    #[rstest::rstest]
    #[test]
    fn cross_process_lock_surfaces_as_coded_error_not_hang() {
        // Given a graph held open by one store (its file locked).
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().to_path_buf();
        let holder = MinigrafStore::new(path.clone(), PathBuf::from("/nonexistent-templates"));
        holder.ensure_dir().expect("ensure_dir");
        holder.create_graph("locked", "test").expect("create");

        // When a fresh store over the same directory opens the same graph
        // (a fresh handle cache — the shape a second process presents; a
        // same-process contender would hit the STG-025 already-open guard,
        // which is the same hard-refusal surface).
        let contender = MinigrafStore::new(path, PathBuf::from("/nonexistent-templates"));
        let result = contender.transact("locked", "(transact [[:a :b \"c\"]])");

        // Then minigraf's coded refusal comes back promptly: an error
        // carrying its stable code, never a hang, never corruption.
        let err = result.expect_err("must refuse");
        let rendered = format!("{err:?}");
        assert!(
            rendered.contains("STG-025") || rendered.contains("STG-026"),
            "expected a coded lock refusal in: {rendered}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn bad_names_are_refused_before_any_filesystem_touch() {
        // Given a store.
        let (_dir, store) = store();

        // When creating with a traversal name.
        let result = store.create_graph("../escape", "test");

        // Then it is refused as a bad name and no file was written.
        let err = result.expect_err("must refuse");
        let rendered = format!("{err:?}");
        assert!(
            rendered.contains("must be 1-64 chars"),
            "expected the name rule in: {rendered}"
        );
        assert!(
            !store.graph_file("../escape").exists()
                || store.graph_file("../escape").extension().is_none()
        );
        let entries: Vec<_> = std::fs::read_dir(&store.minigraf_dir)
            .expect("dir")
            .filter_map(std::result::Result::ok)
            .collect();
        assert!(entries.is_empty(), "no file may be written: {entries:?}");
    }

    #[rstest::rstest]
    #[test]
    fn time_travel_query_reads_as_of_the_first_write() {
        // Given a graph with two transactions.
        let (_dir, store) = store();
        store.create_graph("hist", "test").expect("create");
        let tx1 = store
            .transact("hist", "(transact [[:alice :task/status :todo]])")
            .expect("t1");
        store
            .transact("hist", "(transact [[:alice :task/status :done]])")
            .expect("t2");

        // When querying now and as-of the first write (`:as-of` sits inside
        // the query form, between `:find` and `:where`).
        let now = store
            .query("hist", "(query [:find ?s :where [?e :task/status ?s]])")
            .expect("now");
        let as_of = store
            .query(
                "hist",
                &format!("(query [:find ?s :as-of {tx1} :where [?e :task/status ?s]])"),
            )
            .expect("as-of");

        // Then now reads :done and as-of reads :todo.
        assert!(now.contains("done"), "now: {now}");
        assert!(as_of.contains("todo"), "as_of: {as_of}");
    }

    // -- Templates: instantiate, provenance, replay --------------------------

    /// A templates root with one `stoplight` template (seed + rules) and one
    /// seed-only template (`flat`). Returns the kept-alive TempDir.
    fn templates_root() -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::TempDir::new().expect("tempdir");
        let stoplight = root.path().join("stoplight");
        std::fs::create_dir_all(&stoplight).expect("mkdir");
        std::fs::write(
            stoplight.join("template.dl"),
            concat!(
                "(transact [[:light/1 :state :red]])\n",
                "(rules [ (rule [(green? ?l) [?l :state :green]]) ])\n"
            ),
        )
        .expect("write");
        let flat = root.path().join("flat");
        std::fs::create_dir_all(&flat).expect("mkdir");
        std::fs::write(flat.join("template.dl"), "(transact [[:f/1 :state :on]])").expect("write");
        let path = root.path().to_path_buf();
        (root, path)
    }

    fn store_with_templates() -> (TempDir, TempDir, MinigrafStore) {
        let (troot, tdir) = templates_root();
        let (dir, store) = store();
        let store = store.over_templates(tdir);
        (troot, dir, store)
    }

    /// Renders every `?v` value bound for `attr` as a sorted list of strings
    /// (the JSON shape is `{"columns":[..],"rows":[[..]]}` — rows parsed out).
    fn values(store: &MinigrafStore, graph: &str, attr: &str) -> Vec<String> {
        let json = store
            .query(graph, &format!("(query [:find ?v :where [?e {attr} ?v]])"))
            .expect("query");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("rendered json");
        let mut out: Vec<String> = parsed
            .get("rows")
            .and_then(serde_json::Value::as_array)
            .expect("rows array")
            .iter()
            .filter_map(|row| row.as_array().and_then(|r| r.first()))
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        out.sort();
        out
    }

    #[rstest::rstest]
    #[test]
    fn instantiate_executes_seed_and_rules_and_stamps_provenance() {
        // Given a store over a templates root with a seed+rules template.
        let (_tr, _dir, store) = store_with_templates();

        // When instantiating.
        let outcome = store
            .instantiate_graph("stoplight", "sl-a", "session-a")
            .expect("instantiate");

        // Then the outcome names the graph, template, and creator.
        assert_eq!(outcome.name, "sl-a");
        assert_eq!(outcome.template.as_deref(), Some("stoplight"));
        assert_eq!(outcome.created_by, "session-a");

        // And the seed facts are in the graph.
        assert_eq!(values(&store, "sl-a", ":state"), vec![":red".to_owned()]);

        // And the template's rules were registered at instantiate time.
        store
            .query("sl-a", "(query [:find ?l :where (green? ?l)])")
            .expect("rule registered at instantiate");

        // And the provenance facts are present.
        let prov = values(&store, "sl-a", ":graph/instance-of");
        assert_eq!(prov, vec!["stoplight".to_owned()]);
        assert_eq!(
            values(&store, "sl-a", ":graph/created-by"),
            vec!["session-a".to_owned()]
        );
        assert_eq!(values(&store, "sl-a", ":graph/created-at").len(), 1);
    }

    #[rstest::rstest]
    #[test]
    fn instantiate_rejects_an_unknown_template_without_creating_the_graph() {
        // Given a store with no template named `ghost`.
        let (_tr, _dir, store) = store_with_templates();

        // When instantiating from it.
        let result = store.instantiate_graph("ghost", "sl-a", "session-a");

        // Then it fails naming list_templates and no file was claimed.
        let err = result.expect_err("must refuse");
        let rendered = format!("{err:?}");
        assert!(rendered.contains("list_templates"), "rendered: {rendered}");
        assert!(!store.graph_file("sl-a").exists());
    }

    #[rstest::rstest]
    #[test]
    fn instantiate_rejects_an_invalid_script_before_any_file_is_claimed() {
        // Given a template whose script parses but fails execution (its form
        // is a semantically invalid transact).
        let root = tempfile::TempDir::new().expect("tempdir");
        let broken = root.path().join("broken");
        std::fs::create_dir_all(&broken).expect("mkdir");
        std::fs::write(broken.join("template.dl"), "(transact [[:a :b]])").expect("write");
        let (_dir, store) = store();
        let store = store.over_templates(root.path().to_path_buf());

        // When instantiating from it.
        let result = store.instantiate_graph("broken", "victim", "session-a");

        // Then it fails with the template error and the graph file was never
        // created — validation precedes the claim.
        assert!(result.is_err());
        assert!(!store.graph_file("victim").exists());
    }

    #[rstest::rstest]
    #[test]
    fn instantiate_twice_refuses_and_leaves_the_first_instance_intact() {
        // Given an instantiated template graph with a later fact added.
        let (_tr, _dir, store) = store_with_templates();
        store
            .instantiate_graph("stoplight", "sl-a", "session-a")
            .expect("instantiate");
        store
            .transact("sl-a", "(transact [[:light/9 :state :green]])")
            .expect("user fact");

        // When instantiating the same name again.
        let result = store.instantiate_graph("stoplight", "sl-a", "session-b");

        // Then it is refused and the first instance keeps both its seed fact
        // and the user's later fact.
        assert!(result.is_err());
        assert_eq!(values(&store, "sl-a", ":state").len(), 2);
    }

    #[rstest::rstest]
    #[test]
    fn two_sessions_instantiate_a_template_independently() {
        // Given a store.
        let (_tr, _dir, store) = store_with_templates();

        // When two instances of one template are created.
        store
            .instantiate_graph("stoplight", "sl-a", "session-a")
            .expect("a");
        store
            .instantiate_graph("stoplight", "sl-b", "session-b")
            .expect("b");

        // Then both exist with their own creator provenance.
        assert_eq!(
            values(&store, "sl-a", ":graph/created-by"),
            vec!["session-a".to_owned()]
        );
        assert_eq!(
            values(&store, "sl-b", ":graph/created-by"),
            vec!["session-b".to_owned()]
        );
    }

    #[rstest::rstest]
    #[test]
    fn create_graph_stamps_provenance_and_reports_no_template() {
        // Given a store.
        let (_tr, _dir, store) = store_with_templates();

        // When creating a blank graph with a creator.
        let outcome = store.create_graph("scratch", "session-c").expect("create");

        // Then the provenance records the creator and time, and no template.
        assert_eq!(outcome.name, "scratch");
        assert_eq!(outcome.template, None);
        assert_eq!(outcome.created_by, "session-c");
        assert_eq!(
            values(&store, "scratch", ":graph/created-by"),
            vec!["session-c".to_owned()]
        );
        assert_eq!(values(&store, "scratch", ":graph/instance-of").len(), 0);
    }

    #[rstest::rstest]
    #[test]
    fn attach_binds_an_existing_graph_and_stamps_attach_provenance() {
        // Given an instantiated graph.
        let (_tr, _dir, store) = store_with_templates();
        store
            .instantiate_graph("stoplight", "sl-a", "session-a")
            .expect("instantiate");

        // When a second session attaches to it.
        let outcome = store.attach_graph("sl-a", "session-b").expect("attach");

        // Then the provenance names the template and the attaching session,
        // and the graph still serves its facts.
        assert_eq!(outcome.name, "sl-a");
        assert_eq!(outcome.template.as_deref(), Some("stoplight"));
        assert_eq!(outcome.created_by, "session-b");
        assert_eq!(
            values(&store, "sl-a", ":graph/attached-by"),
            vec!["session-b".to_owned()]
        );
    }

    #[rstest::rstest]
    #[test]
    fn attach_of_an_unknown_graph_names_instantiate_or_attach() {
        // Given a store with no such graph.
        let (_tr, _dir, store) = store_with_templates();

        // When attaching.
        let result = store.attach_graph("ghost", "session-b");

        // Then the refusal names both remedies.
        let err = result.expect_err("must refuse");
        let rendered = format!("{err:?}");
        assert!(
            rendered.contains("instantiate_graph") && rendered.contains("attach_graph"),
            "rendered: {rendered}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn cross_process_lock_surfaces_cleanly_from_attach() {
        // Given a graph file held by another store.
        let (_tr, dir, store) = store_with_templates();
        let holder = MinigrafStore::new(
            dir.path().to_path_buf(),
            PathBuf::from("/nonexistent-templates"),
        );
        holder.create_graph("held", "other").expect("create");

        // When attaching from a fresh store (fresh handle cache).
        let contender = store.over_templates(PathBuf::from("/nonexistent-templates"));
        let result = contender.attach_graph("held", "session-b");

        // Then minigraf's coded lock refusal comes back promptly.
        let err = result.expect_err("must refuse");
        let rendered = format!("{err:?}");
        assert!(
            rendered.contains("STG-025") || rendered.contains("STG-026"),
            "rendered: {rendered}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn replay_on_reopen_runs_rules_only_and_never_touches_seed_timestamps() {
        // Given an instantiated graph whose seed stamp carries a timestamp,
        // dropped after writing so its handle is gone.
        let dir = TempDir::new().expect("tempdir");
        let (troot, tdir) = templates_root();
        let path = dir.path().to_path_buf();
        {
            let store = MinigrafStore::new(path.clone(), tdir.clone());
            store
                .instantiate_graph("stoplight", "sl-a", "session-a")
                .expect("instantiate");
            store
                .transact("sl-a", "(retract [[:light/1 :state :red]])")
                .expect("clear the seed state");
            store
                .transact("sl-a", "(transact [[:light/1 :state :green]])")
                .expect("move the light");
        }

        // When a fresh store over the same directory opens the same graph
        // (the cache-miss replay path).
        let store = MinigrafStore::new(path, tdir);
        let json = store
            .query("sl-a", "(query [:find ?s :where [?e :state ?s]])")
            .expect("query");

        // Then the rules compute again without re-registration...
        assert!(json.contains("green"), "state: {json}");
        store
            .query("sl-a", "(query [:find ?l :where (green? ?l)])")
            .expect("rule survived replay");

        // ...and the seed replayed exactly once ever: the created-at stamp
        // holds one value, not a second one from the reopen.
        assert_eq!(values(&store, "sl-a", ":graph/created-at").len(), 1);
        // And the seed's original fact was not re-asserted over the user's
        // later write (state stays green — replaying :red would have added a
        // second value).
        assert_eq!(values(&store, "sl-a", ":state"), vec![":green".to_owned()]);
        drop(troot);
    }

    #[rstest::rstest]
    #[test]
    fn replay_with_a_deleted_template_still_opens_and_warns() {
        // Given an instantiated graph whose template is deleted before the
        // reopen.
        let dir = TempDir::new().expect("tempdir");
        let troot = tempfile::TempDir::new().expect("tempdir");
        let stoplight = troot.path().join("stoplight");
        std::fs::create_dir_all(&stoplight).expect("mkdir");
        std::fs::write(
            stoplight.join("template.dl"),
            "(transact [[:light/1 :state :red]])\n(rules [ (rule [(green? ?l) [?l :state :green]]) ])\n",
        )
        .expect("write");
        let tdir = troot.path().to_path_buf();
        let path = dir.path().to_path_buf();
        {
            let store = MinigrafStore::new(path.clone(), tdir.clone());
            store
                .instantiate_graph("stoplight", "sl-a", "session-a")
                .expect("instantiate");
        }
        std::fs::remove_dir_all(&stoplight).expect("delete template");

        // When a fresh store opens the graph.
        let store = MinigrafStore::new(path, tdir);

        // Then the open degrades: the fact reads back and the tool result
        // carries the one-line unavailable-template warning.
        let json = store
            .query("sl-a", "(query [:find ?s :where [?e :state ?s]])")
            .expect("graph still opens");
        assert!(json.contains("red"), "state: {json}");
        let warned = store.warning_for("sl-a").expect("warning present");
        assert!(
            warned.contains("stoplight") && warned.contains("unavailable"),
            "warning: {warned}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn an_old_format_graph_without_provenance_opens_clean_with_no_replay() {
        // Given a graph created before templates existed: facts only, no
        // provenance (written by hand through minigraf).
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().to_path_buf();
        {
            let store = MinigrafStore::new(path.clone(), PathBuf::from("/nonexistent-templates"));
            store.ensure_dir().expect("ensure_dir");
            std::fs::File::create(store.graph_file("legacy")).expect("claim");
            let db = minigraf::Minigraf::open(store.graph_file("legacy")).expect("open");
            db.execute("(transact [[:old :thing true]])").expect("seed");
        }

        // When a fresh store opens it and queries.
        let store = MinigrafStore::new(path, PathBuf::from("/nonexistent-templates"));
        let json = store
            .query("legacy", "(query [:find ?t :where [?e :thing ?t]])")
            .expect("query");

        // Then the fact reads back and no replay warning exists.
        assert!(json.contains("true"), "json: {json}");
        assert_eq!(store.warning_for("legacy"), None);
    }

    #[rstest::rstest]
    #[test]
    fn attach_of_an_already_open_graph_hits_the_cache_not_a_second_open() {
        // Given a graph the store already holds open.
        let (_tr, _dir, store) = store_with_templates();
        store
            .instantiate_graph("stoplight", "sl-a", "session-a")
            .expect("instantiate");
        let handles_before = {
            let handles = store.handles.lock();
            handles.len()
        };

        // When attaching again from the same store.
        store.attach_graph("sl-a", "session-b").expect("attach");

        // Then the handle count did not grow — the cache was reused (a second
        // in-process open would be STG-025).
        let handles_after = {
            let handles = store.handles.lock();
            handles.len()
        };
        assert_eq!(handles_after, handles_before);
    }
}
