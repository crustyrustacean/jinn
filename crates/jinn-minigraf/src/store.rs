//! The graph store: one minigraf handle per named graph, held for the
//! server's life, plus the four tool operations behind it.
//!
//! All methods are synchronous — the tools call them from
//! [`tokio::task::spawn_blocking`](tokio::task::spawn_blocking). Every method
//! is auto-commit: one `execute` per call, which minigraf already makes an
//! atomic WAL transaction. Explicit `WriteTransaction`s are deliberately not
//! used: they are `!Send` and poison same-thread `execute` while open
//! (`Int001`).

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use error_stack::Report;
use parking_lot::Mutex;

use crate::errors::MinigrafServerError;
use crate::json_rows;
use crate::naming;

/// One minigraf handle per graph name, plus the directory the `.graph` files
/// live in. Cheap to clone; every clone sees the same handle cache.
#[derive(Clone)]
pub struct MinigrafStore {
    minigraf_dir: PathBuf,
    handles: Arc<Mutex<HashMap<String, minigraf::Minigraf>>>,
}

impl MinigrafStore {
    /// Builds a store over `minigraf_dir` without touching the filesystem;
    /// call [`ensure_dir`](Self::ensure_dir) to create the directory.
    #[must_use]
    pub fn new(minigraf_dir: PathBuf) -> Self {
        Self {
            minigraf_dir,
            handles: Arc::new(Mutex::new(HashMap::new())),
        }
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

    /// Creates `<name>.graph`; fails if it already exists, leaving the
    /// existing file untouched.
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
    pub fn create_graph(&self, name: &str) -> Result<(), Report<MinigrafServerError>> {
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
        self.handles.lock().insert(name.to_owned(), db);
        Ok(())
    }

    /// Runs one Datalog write command against an existing graph; returns the
    /// transaction id (unix millis — the `:as-of` value for reads after this
    /// write).
    ///
    /// # Errors
    ///
    /// [`MinigrafServerError::UnknownGraph`] when the graph was never created (the
    /// remedy is `create_graph`), [`MinigrafServerError::BadName`] for an unsafe
    /// name, [`MinigrafServerError::Minigraf`] for parse or execution failures.
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
    /// [`MinigrafServerError::UnknownGraph`] when the graph was never created (the
    /// remedy is `create_graph`), [`MinigrafServerError::BadName`] for an unsafe
    /// name, [`MinigrafServerError::Minigraf`] for parse or execution failures.
    pub fn query(&self, graph: &str, datalog: &str) -> Result<String, Report<MinigrafServerError>> {
        let db = self.existing_handle(graph)?;
        let result = db
            .execute(datalog)
            .map_err(|e| Report::new(MinigrafServerError::Minigraf(e.to_string())))?;
        Ok(json_rows::render(&result))
    }

    /// Names of the graphs in the directory, sorted.
    ///
    /// # Errors
    ///
    /// [`MinigrafServerError::StoreDir`] when the directory cannot be read; an empty
    /// directory is an empty list, not an error.
    pub fn list_graphs(&self) -> Result<Vec<String>, Report<MinigrafServerError>> {
        let mut names = Vec::new();
        let entries = fs::read_dir(&self.minigraf_dir).map_err(|e| {
            Report::new(MinigrafServerError::StoreDir(format!(
                "{}: {e}",
                self.minigraf_dir.display()
            )))
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "graph")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                names.push(stem.to_owned());
            }
        }
        names.sort();
        Ok(names)
    }

    /// Resolves the handle for a graph that must already exist.
    ///
    /// Cache hit returns the held handle; a miss checks the file exists
    /// first — a plain minigraf open would *create* the file, which would
    /// silently violate create-before-use.
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
        self.handles.lock().insert(name.to_owned(), db.clone());
        Ok(db)
    }

    fn graph_file(&self, name: &str) -> PathBuf {
        self.minigraf_dir.join(format!("{name}.graph"))
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
        let store = MinigrafStore::new(dir.path().to_path_buf());
        store.ensure_dir().expect("ensure_dir");
        (dir, store)
    }

    #[rstest::rstest]
    #[test]
    fn create_then_transact_then_query_roundtrips() {
        // Given a store with a created graph.
        let (_dir, store) = store();
        store.create_graph("t1").expect("create");

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
        store.create_graph("dup").expect("create");
        store
            .transact("dup", "(transact [[:alice :task/name \"original\"]])")
            .expect("transact");

        // When creating the same name again.
        let result = store.create_graph("dup");

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
            rendered.contains("create_graph first"),
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
        store.create_graph("cached").expect("create");
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
    fn list_graphs_lists_created_graphs_sorted() {
        // Given graphs created out of order.
        let (_dir, store) = store();
        store.create_graph("beta").expect("create");
        store.create_graph("alpha").expect("create");

        // When listing.
        let names = store.list_graphs().expect("list");

        // Then names come back sorted.
        assert_eq!(names, vec!["alpha".to_owned(), "beta".to_owned()]);
    }

    #[rstest::rstest]
    #[test]
    fn list_graphs_ignores_non_graph_files() {
        // Given a directory with a stray non-.graph file.
        let (_dir, store) = store();
        store.create_graph("real").expect("create");
        fs::write(store.minigraf_dir.join("notes.txt"), "not a graph").expect("stray");

        // When listing.
        let names = store.list_graphs().expect("list");

        // Then only the .graph stem is listed.
        assert_eq!(names, vec!["real".to_owned()]);
    }

    #[rstest::rstest]
    #[test]
    fn persistence_survives_handle_drop_and_reopen() {
        // Given a graph written in one store (its handles dropped with it).
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().to_path_buf();
        {
            let store = MinigrafStore::new(path.clone());
            store.ensure_dir().expect("ensure_dir");
            store.create_graph("durable").expect("create");
            store
                .transact("durable", "(transact [[:alice :task/name \"survives\"]])")
                .expect("transact");
        }

        // When a fresh store over the same directory opens the same graph.
        let store = MinigrafStore::new(path);
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
        let store = MinigrafStore::new(dir.path().to_path_buf());
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
        let holder = MinigrafStore::new(path.clone());
        holder.ensure_dir().expect("ensure_dir");
        holder.create_graph("locked").expect("create");

        // When a fresh store over the same directory opens the same graph
        // (a fresh handle cache — the shape a second process presents; a
        // same-process contender would hit the STG-025 already-open guard,
        // which is the same hard-refusal surface).
        let contender = MinigrafStore::new(path);
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
        let result = store.create_graph("../escape");

        // Then it is refused as a bad name and no file was written.
        let err = result.expect_err("must refuse");
        let rendered = format!("{err:?}");
        assert!(
            rendered.contains("must be 1-64 chars"),
            "expected the name rule in: {rendered}"
        );
        assert_eq!(store.list_graphs().expect("list"), Vec::<String>::new());
    }

    #[rstest::rstest]
    #[test]
    fn time_travel_query_reads_as_of_the_first_write() {
        // Given a graph with two transactions.
        let (_dir, store) = store();
        store.create_graph("hist").expect("create");
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
        assert!(as_of.contains("todo"), "as-of: {as_of}");
    }
}
