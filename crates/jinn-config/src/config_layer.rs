//! The `ConfigLayer` — `jinn.toml` as one in-memory, reloadable,
//! read+write document.
//!
//! A [`ConfigLayer`] is a cheap `Clone` handle over a shared in-memory
//! [`DocumentMut`]. Every clone reads the same live snapshot, so a
//! consumer holds the handle and reads through it at the point of use
//! rather than caching a value: a [`reload`] makes an externally-edited
//! `jinn.toml` visible to every subsequent read with no restart and no
//! propagation message.
//!
//! Reads are typed through [`Configurable`], so nothing outside a
//! section's own `impl` names its key. Writes go through the
//! comment-preserving [`DocumentPatcher`], so a save keeps the user's
//! comments, key order, sibling sections, and any table this layer does
//! not model.

use std::path::PathBuf;
use std::sync::Arc;

use error_stack::{Report, ResultExt};
use jinn_common::toml_patch::{DocumentPatcher, PatchError};
use parking_lot::RwLock;
use toml_edit::{DocumentMut, Item, Table};

use crate::configurable::{ConfigList, ConfigSectionError, Configurable};

/// The configuration layer: a live, typed, read+write view of the
/// configuration document.
///
/// Consumers read at the point of use and never cache: the handle is
/// cheap to clone and every read observes the current snapshot.
#[derive(Debug, Clone)]
pub struct ConfigLayer {
    inner: Arc<ConfigInner>,
}

/// The shared state behind every [`ConfigLayer`] handle.
struct ConfigInner {
    /// The decor-preserving document; the authoritative in-memory state.
    doc: RwLock<DocumentMut>,
    /// Where the document is persisted, for `reload` and `put`.
    storage: RwLock<Arc<dyn ConfigDocumentStorage>>,
    /// Sections registered for `validate()`, in registration order.
    registry: RwLock<Vec<RegisteredSection>>,
}

impl std::fmt::Debug for ConfigInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigInner")
            .field("backend", &self.storage.read().name())
            .field("registered_sections", &self.registry.read().len())
            .finish_non_exhaustive()
    }
}

/// One registered section, captured type-erased so `validate` can
/// fail-fast on a malformed table without naming the type.
#[derive(Clone)]
struct RegisteredSection {
    key: &'static str,
    type_name: &'static str,
    check: SectionCheck,
}

/// The type-erased "does this table deserialize" check a registered
/// section carries, so `validate` never has to name the section's type.
type SectionCheck = Arc<dyn Fn(&toml::Table) -> Result<(), ConfigSectionError> + Send + Sync>;

impl std::fmt::Debug for RegisteredSection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisteredSection")
            .field("key", &self.key)
            .field("type_name", &self.type_name)
            .finish_non_exhaustive()
    }
}

/// Why a config document could not be loaded or saved.
#[derive(Debug, wherror::Error)]
pub enum ConfigError {
    /// The backing store could not be read or written.
    #[error("config document storage failed: {detail}")]
    Storage {
        /// The underlying failure.
        detail: String,
    },
    /// The document on disk is not valid TOML.
    #[error("config document is not valid TOML: {detail}")]
    Malformed {
        /// The parser's complaint.
        detail: String,
    },
    /// A section's value could not be serialized back to TOML.
    #[error("config section could not be serialized: {detail}")]
    Serialize {
        /// The serializer's complaint.
        detail: String,
    },
    /// A section's value did not fit the shape the document expects.
    #[error("config section [{key}] is not a table")]
    NotATable {
        /// The section's dotted key.
        key: &'static str,
    },
    /// The patcher could not apply the new value onto the document.
    #[error("config document could not be patched: {detail}")]
    Patch {
        /// The patcher's complaint.
        detail: String,
    },
}

/// Reads and writes the configuration document.
///
/// The trait is the seam that keeps tests off a real `~/.config`: the
/// in-memory backend serves a document the test owns, so a `put` can be
/// asserted byte-for-byte without touching disk.
pub trait ConfigDocumentStorage: Send + Sync + 'static {
    /// The backend's name, for debugging.
    fn name(&self) -> &'static str;

    /// Reads and parses the document.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Storage`] when the read fails, and
    /// [`ConfigError::Malformed`] when the bytes are not valid TOML.
    fn read(&self) -> Result<DocumentMut, Report<ConfigError>>;

    /// Persists the document.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Storage`] when the write fails.
    fn write(&self, doc: &DocumentMut) -> Result<(), Report<ConfigError>>;
}

/// The production backend: the document lives at a path on disk.
#[derive(Debug)]
pub struct FilesystemConfigStorage {
    path: PathBuf,
}

impl FilesystemConfigStorage {
    /// A backend over the user's `jinn.toml` in the platform config dir.
    #[must_use]
    pub fn default_path() -> Self {
        Self {
            path: jinn_common::app_paths::AppPaths::default().preferences_path(),
        }
    }

    /// Points a backend at the document's path.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The document's path.
    #[must_use]
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl ConfigDocumentStorage for FilesystemConfigStorage {
    fn name(&self) -> &'static str {
        "FilesystemConfigStorage"
    }

    fn read(&self) -> Result<DocumentMut, Report<ConfigError>> {
        // A missing document is a fresh install, not a failure: the
        // caller materializes an empty one and every section reads its
        // default.
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Ok(DocumentMut::new());
        };
        text.parse::<DocumentMut>()
            .change_context(ConfigError::Malformed {
                detail: format!("{} is not valid TOML", self.path.display()),
            })
    }

    fn write(&self, doc: &DocumentMut) -> Result<(), Report<ConfigError>> {
        std::fs::write(&self.path, doc.to_string()).change_context(ConfigError::Storage {
            detail: format!("failed to write {}", self.path.display()),
        })
    }
}

/// The test backend: the document lives in the process.
#[derive(Debug, Default)]
pub struct InMemoryConfigStorage {
    doc: RwLock<DocumentMut>,
    /// When set, `read` fails as if the document were corrupt — the
    /// seam a test uses to drive the failure paths that a real file
    /// would otherwise need a deliberately malformed write to reach.
    broken: RwLock<bool>,
}

impl InMemoryConfigStorage {
    /// Seeds a backend with a document.
    #[must_use]
    pub fn new(doc: DocumentMut) -> Self {
        Self {
            doc: RwLock::new(doc),
            broken: RwLock::new(false),
        }
    }

    /// The document's current text, for asserting what a save wrote.
    #[must_use]
    pub fn text(&self) -> String {
        self.doc.read().to_string()
    }

    /// Makes `read` fail as if the document were unparseable.
    pub fn set_broken(&self, broken: bool) {
        *self.broken.write() = broken;
    }
}

impl ConfigDocumentStorage for InMemoryConfigStorage {
    fn name(&self) -> &'static str {
        "InMemoryConfigStorage"
    }

    fn read(&self) -> Result<DocumentMut, Report<ConfigError>> {
        if *self.broken.read() {
            return Err(Report::new(ConfigError::Malformed {
                detail: "in-memory document is marked broken".to_owned(),
            }));
        }
        Ok(self.doc.read().clone())
    }

    fn write(&self, doc: &DocumentMut) -> Result<(), Report<ConfigError>> {
        *self.doc.write() = doc.clone();
        Ok(())
    }
}

impl ConfigLayer {
    /// Loads the document through `storage` and returns the layer.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Storage`] or [`ConfigError::Malformed`]
    /// when the document cannot be read or parsed.
    pub fn load(storage: Arc<dyn ConfigDocumentStorage>) -> Result<Self, Report<ConfigError>> {
        let doc = storage.read()?;
        Ok(Self {
            inner: Arc::new(ConfigInner {
                doc: RwLock::new(doc),
                storage: RwLock::new(storage),
                registry: RwLock::new(Vec::new()),
            }),
        })
    }

    /// The backend's name, for debugging.
    #[must_use]
    pub fn backend_name(&self) -> &'static str {
        self.inner.storage.read().name()
    }

    /// The current document's text, as the layer holds it.
    #[must_use]
    pub fn document_text(&self) -> String {
        self.inner.doc.read().to_string()
    }

    /// Registers a section for launch-time fail-fast validation.
    ///
    /// Registering is how a section opts into [`Self::validate`]. A
    /// section nobody registered is simply never validated — reads of it
    /// still fail loudly, just at the read rather than at launch.
    pub fn register<T: Configurable>(&self) -> &Self {
        let check = |table: &toml::Table| T::from_table(table).map(|_| ());
        self.inner.registry.write().push(RegisteredSection {
            key: T::KEY,
            type_name: std::any::type_name::<T>(),
            check: Arc::new(check),
        });
        self
    }

    /// Reads the live value of `T`'s section.
    ///
    /// The document's table is cloned out under the read guard and
    /// deserialized outside it, so a slow section never holds the lock
    /// against a concurrent writer.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigSectionError::NotATable`] when a path segment is
    /// not a table, and [`ConfigSectionError::Malformed`] when the
    /// table does not deserialize. An absent section at any depth is
    /// *not* an error — it reads `T::default()`.
    pub fn get<T: Configurable>(&self) -> Result<T, ConfigSectionError> {
        let doc = self.inner.doc.read().clone();
        let Some(table) = section_table(&doc, T::KEY)? else {
            return Ok(T::default());
        };
        T::from_table(&table)
    }

    /// Reads the live value of `T`'s section, falling back to
    /// `T::default()` when the section is absent or malformed.
    ///
    /// For a reader that has no sensible reaction to a bad file. A
    /// malformed section is a launch-time error caught by
    /// [`Self::validate`] — by the time a render frame or a tool call
    /// reads, the user has a running app and is better served by the
    /// documented default than by a blank pane. Use [`Self::get`] where
    /// the failure should surface.
    pub fn read<T: Configurable>(&self) -> T {
        self.get::<T>().unwrap_or_default()
    }

    /// Writes `value` over `T`'s section and persists the document.
    ///
    /// A key the section's new value no longer declares is removed from
    /// the section's own table first: the patcher only walks keys the new
    /// value names, so without this a field dropped from a type would
    /// linger on disk while memory disagreed with it. Removal is scoped
    /// to the section, so a sibling section or an unmodelled table is
    /// never touched.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Serialize`] when the value cannot be
    /// serialized, [`ConfigError::Patch`] when the patcher cannot apply
    /// it, and [`ConfigError::Storage`] when the write fails.
    pub fn put<T: Configurable>(&self, value: &T) -> Result<(), Report<ConfigError>> {
        let serialized = toml::Value::try_from(value).change_context(ConfigError::Serialize {
            detail: format!("section [{}] did not serialize", T::KEY),
        })?;
        let toml::Value::Table(section) = serialized else {
            return Err(Report::new(ConfigError::NotATable { key: T::KEY }));
        };

        let mut doc = self.inner.doc.read().clone();
        remove_stale_keys(&mut doc, T::KEY, &section);

        let mut patcher = DocumentPatcher::new();
        // Every table a section carries is a named section and keeps header
        // form; the nested arrays it may hold are the section's own values and
        // render inline.
        //
        // Registered per *prefix*, not just for the full key: `put` hands the
        // patcher a document whose root carries `watchdog` wrapping `stall`
        // wrapping `stall.enabled`, and each level is a real section needing
        // its own header. Registering only `watchdog.stall` would leave
        // `watchdog` unregistered, and an unregistered table is by definition
        // a value — which would inline the whole subtree.
        //
        // The wrapped tree is `{watchdog: {stall: <WatchdogCfg>}}`, and
        // `WatchdogCfg` itself has a `stall` field, so the path a sub-table
        // of the section reaches is one segment longer than the section's own
        // key. Each level up to and including that leaf is registered; a
        // deeper table is a field *inside* the section and is one of its
        // values, which renders inline.
        for (depth, _) in T::KEY.split('.').enumerate() {
            let prefix: Vec<&'static str> = T::KEY.split('.').take(depth + 1).collect();
            patcher.register_section(prefix);
        }
        if let Some(entry) = T::ENTRY_KEY {
            patcher.register_array_key(entry.full_path(T::KEY), entry.field());
        }
        patcher
            .apply(&wrap_at(T::KEY, section), doc.as_table_mut())
            .change_context(PatchError::Generic)
            .change_context(ConfigError::Patch {
                detail: format!("section [{}] did not apply", T::KEY),
            })?;

        // Disk first, then memory: a failed write leaves the in-memory
        // document agreeing with what is actually on disk.
        self.inner.storage.read().write(&doc)?;
        *self.inner.doc.write() = doc;
        Ok(())
    }

    /// Reads the live value of a [`ConfigList`] section.
    ///
    /// The counterpart of [`Self::get`] for a section that is a bare
    /// array of tables. An absent list is not an error — it reads
    /// `Vec::default()`.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigSectionError::NotATable`] when a path segment is
    /// not a table, and [`ConfigSectionError::Malformed`] when an entry
    /// does not deserialize into `T`.
    pub fn get_list<T: ConfigList>(&self) -> Result<Vec<T>, ConfigSectionError> {
        let doc = self.inner.doc.read().clone();
        let Some(arrays) = section_arrays(&doc, T::KEY)? else {
            return Ok(Vec::default());
        };
        deserialize_entries(T::KEY, arrays)
    }

    /// Writes `value` over a [`ConfigList`] section and persists the
    /// document.
    ///
    /// Entries are matched by [`ConfigList::ENTRY_KEY`], so rewriting one
    /// entry leaves its siblings — and their per-entry comments —
    /// untouched.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Serialize`] when the value cannot be
    /// serialized, [`ConfigError::Patch`] when the patcher cannot apply
    /// it, and [`ConfigError::Storage`] when the write fails.
    pub fn put_list<T: ConfigList>(&self, value: &[T]) -> Result<(), Report<ConfigError>> {
        let serialized = toml::Value::try_from(value).change_context(ConfigError::Serialize {
            detail: format!("section [{}] did not serialize", T::KEY),
        })?;
        let toml::Value::Array(entries) = serialized else {
            return Err(Report::new(ConfigError::NotATable { key: T::KEY }));
        };

        let mut doc = self.inner.doc.read().clone();

        // Resolve the list's parent table, then hand the patcher a table
        // carrying the entries under the list's own leaf key.
        //
        // The array-key registry is consulted with paths relative to
        // whatever table `apply` is handed, so a nested list registers
        // the LEAF alone. Registering the root-relative path here would
        // never match, and the patcher would then coerce the array into
        // inline form — mangling the document.
        let (leaf, parent_path): (String, Vec<&str>) = match T::KEY.rsplit_once('.') {
            Some((head, leaf)) => (leaf.to_owned(), head.split('.').collect()),
            None => (T::KEY.to_owned(), Vec::new()),
        };
        let mut patcher = DocumentPatcher::new();
        patcher.register_array_key([static_leaf::<T>()], T::ENTRY_KEY);

        let parent = ensure_table(&mut doc, &parent_path)
            .change_context(PatchError::Generic)
            .change_context(ConfigError::Patch {
                detail: format!("section [{}] did not apply", T::KEY),
            })?;
        let mut list_value = toml::value::Table::new();
        list_value.insert(leaf.clone(), toml::Value::Array(entries));
        patcher
            .apply(&list_value, parent)
            .change_context(PatchError::Generic)
            .change_context(ConfigError::Patch {
                detail: format!("section [{}] did not apply", T::KEY),
            })?;

        // The patcher matches by entry key rather than replacing the
        // array, so an entry the caller dropped must go explicitly.
        drop_unmatched_entries(&mut doc, T::KEY, T::ENTRY_KEY, value);

        self.inner.storage.read().write(&doc)?;
        *self.inner.doc.write() = doc;
        Ok(())
    }

    /// Walks every registered section, failing on the first malformed
    /// one in registration order.
    ///
    /// Run once at startup: a section whose table is present but does not
    /// deserialize aborts the launch naming the section, instead of
    /// surfacing as a mysterious wrong value much later. An absent
    /// section is fine — it reads its default.
    ///
    /// # Errors
    ///
    /// Returns the first [`ConfigSectionError`] a registered section
    /// reports.
    pub fn validate(&self) -> Result<(), ConfigSectionError> {
        let doc = self.inner.doc.read().clone();
        let registry = self.inner.registry.read().clone();
        for section in &registry {
            if let Some(table) = section_table(&doc, section.key)? {
                (section.check)(&table)?;
            }
        }
        Ok(())
    }

    /// Re-reads the document and atomically replaces the in-memory
    /// snapshot.
    ///
    /// A reader sees either the whole old document or the whole new one,
    /// never a mix. On a parse failure the existing document is left
    /// untouched and the error returned — a typo in the file must not
    /// blank the running program's configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Storage`] or [`ConfigError::Malformed`]
    /// when the document cannot be re-read or parsed.
    pub fn reload(&self) -> Result<(), Report<ConfigError>> {
        let doc = self.inner.storage.read().read()?;
        *self.inner.doc.write() = doc;
        Ok(())
    }

    /// Points the layer at a different storage backend and re-reads from
    /// it. For tests that seed a document over an already-built layer;
    /// production never swaps the backend out from under itself.
    ///
    /// The swap and the re-read are one step on purpose: a caller that
    /// swapped and then reloaded separately could interleave with another
    /// reader and leave the snapshot describing the *previous* backend.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when the new backend cannot be read.
    pub fn use_storage(
        &self,
        storage: Arc<dyn ConfigDocumentStorage>,
    ) -> Result<(), Report<ConfigError>> {
        let doc = storage.read()?;
        *self.inner.storage.write() = storage;
        *self.inner.doc.write() = doc;
        Ok(())
    }
}

/// Resolves a section's table by its dotted key.
///
/// `Ok(None)` means the section is absent at some depth, which reads as
/// the section's default. `Err` means a segment exists but is not a
/// table — a shape disagreement between the document and the section's
/// declared key, which must not be confused with mere absence.
fn section_table(
    doc: &DocumentMut,
    key: &'static str,
) -> Result<Option<toml::Table>, ConfigSectionError> {
    let mut cursor = doc.as_table();
    for segment in key.split('.') {
        let Some(item) = cursor.get(segment) else {
            return Ok(None);
        };
        cursor = item
            .as_table()
            .ok_or_else(|| ConfigSectionError::NotATable {
                key,
                segment: segment.to_owned(),
            })?;
    }
    // `toml_edit`'s table keeps comments and key order and is a distinct
    // type from `toml`'s. Converting to the data-only shape the
    // deserializer wants drops the decor without re-parsing the file.
    let value = item_to_value(&Item::Table(cursor.clone()));
    match value {
        toml::Value::Table(table) => Ok(Some(table)),
        _ => Err(ConfigSectionError::NotATable {
            key,
            segment: key.to_owned(),
        }),
    }
}

/// Resolves a section's entries by its dotted key, for a
/// [`ConfigList`] section.
///
/// `Ok(None)` means the list is absent, which reads as empty. A segment
/// that is not a table is a shape disagreement and is reported as such.
fn section_arrays(
    doc: &DocumentMut,
    key: &'static str,
) -> Result<Option<Vec<toml::Table>>, ConfigSectionError> {
    let mut cursor = doc.as_table();
    let mut segments: Vec<&str> = key.split('.').collect();
    let Some(leaf) = segments.pop() else {
        return Ok(None);
    };
    for segment in &segments {
        let Some(item) = cursor.get(segment) else {
            return Ok(None);
        };
        cursor = item
            .as_table()
            .ok_or_else(|| ConfigSectionError::NotATable {
                key,
                segment: (*segment).to_owned(),
            })?;
    }
    let Some(item) = cursor.get(leaf) else {
        return Ok(None);
    };
    let toml::Value::Array(entries) = item_to_value(item) else {
        return Err(ConfigSectionError::NotATable {
            key,
            segment: leaf.to_owned(),
        });
    };
    let tables = entries
        .into_iter()
        .map(|entry| match entry {
            toml::Value::Table(table) => Ok(table),
            _ => Err(ConfigSectionError::NotATable {
                key,
                segment: leaf.to_owned(),
            }),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(tables))
}

/// Deserializes a list section's entries into `T`.
fn deserialize_entries<T: ConfigList>(
    key: &'static str,
    tables: Vec<toml::Table>,
) -> Result<Vec<T>, ConfigSectionError> {
    tables
        .into_iter()
        .map(|table| {
            T::deserialize(table).map_err(|err| ConfigSectionError::Malformed {
                key,
                detail: err.message().to_owned(),
            })
        })
        .collect()
}

/// Walks (creating as needed) to the table at `path` in `doc`.
///
/// A missing intermediate table is created with no decor, so the patcher
/// inserts the child in header form and no comment is stranded.
fn ensure_table<'d>(
    doc: &'d mut DocumentMut,
    path: &[&str],
) -> Result<&'d mut toml_edit::Table, PatchError> {
    let mut cursor = doc.as_table_mut();
    for segment in path {
        let entry = cursor
            .entry(segment)
            .or_insert(Item::Table(toml_edit::Table::new()));
        cursor = entry.as_table_mut().ok_or(PatchError::Generic)?;
    }
    Ok(cursor)
}

/// Removes array-of-tables entries at `key` whose `entry_field` value is
/// not present in `keep`.
///
/// The patcher matches a registered array by its entry key and leaves
/// unmatched entries alone, which is what preserves a sibling's comment.
/// The flip side is that an entry the caller *deleted* would survive
/// forever, so removal is explicit here.
fn drop_unmatched_entries<T: ConfigList>(
    doc: &mut DocumentMut,
    key: &str,
    entry_field: &str,
    keep: &[T],
) {
    let wanted: Vec<toml::Value> = keep
        .iter()
        .filter_map(|entry| toml::Value::try_from(entry).ok())
        .filter_map(|entry| {
            let table = entry.as_table()?;
            table.get(entry_field).cloned()
        })
        .collect();
    let Some((head, leaf)) = key.rsplit_once('.') else {
        return;
    };
    let Some(parent) = resolve_table_mut(doc.as_table_mut(), &head.split('.').collect::<Vec<_>>())
    else {
        return;
    };
    let Some(Item::ArrayOfTables(array)) = parent.get_mut(leaf) else {
        return;
    };
    array.retain(|entry| {
        entry
            .get(entry_field)
            .is_some_and(|value| wanted.iter().any(|want| item_to_value(value) == *want))
    });
}

/// Converts a `toml_edit` item into the data-only `toml` value.
///
/// Written out by hand rather than routed through a second TOML parse:
/// this runs on every read, including the render path's per-frame reads.
/// `toml_edit` 0.22 offers no conversion of its own — its `serde` feature
/// is deserialization-only.
fn item_to_value(item: &Item) -> toml::Value {
    match item {
        Item::None => toml::Value::Table(toml::value::Table::new()),
        Item::Value(value) => value_to_data(value),
        Item::Table(table) => toml::Value::Table(
            table
                .iter()
                .map(|(key, child)| (key.to_owned(), item_to_value(child)))
                .collect(),
        ),
        Item::ArrayOfTables(array) => {
            let entries: Vec<toml::Value> = array
                .iter()
                .map(|table| item_to_value(&Item::Table(table.clone())))
                .collect();
            toml::Value::Array(entries)
        }
    }
}

/// Converts a single `toml_edit` scalar/inline value, preserving its
/// value and dropping only the surrounding decor.
fn value_to_data(value: &toml_edit::Value) -> toml::Value {
    match value {
        toml_edit::Value::String(inner) => toml::Value::String(inner.value().clone()),
        toml_edit::Value::Integer(inner) => toml::Value::Integer(*inner.value()),
        toml_edit::Value::Float(inner) => toml::Value::Float(*inner.value()),
        toml_edit::Value::Boolean(inner) => toml::Value::Boolean(*inner.value()),
        toml_edit::Value::Datetime(inner) => toml::Value::Datetime(*inner.value()),
        toml_edit::Value::Array(inner) => {
            let entries: Vec<toml::Value> = inner.iter().map(value_to_data).collect();
            toml::Value::Array(entries)
        }
        toml_edit::Value::InlineTable(inner) => toml::Value::Table(
            inner
                .iter()
                .map(|(key, child)| (key.to_owned(), value_to_data(child)))
                .collect(),
        ),
    }
}

/// Nests `section` under `key`'s dotted path, producing the wrapper the
/// patcher applies at the document root.
///
/// Applying at the root rather than at a resolved parent table is what
/// lets the patcher's registry keep using document-root-relative paths,
/// and the patcher creates any missing intermediate table itself — with
/// no decor, so a newly created umbrella is bare.
fn wrap_at(key: &str, section: toml::value::Table) -> toml::value::Table {
    key.split('.')
        .rev()
        .fold(toml::Value::Table(section), |inner, segment| {
            let mut table = toml::value::Table::new();
            table.insert(segment.to_owned(), inner);
            toml::Value::Table(table)
        })
        .try_into()
        .unwrap_or_else(|_| toml::value::Table::new())
}

/// Removes keys under `key` that the section's new value no longer
/// declares, leaving every other key of that table alone.
fn remove_stale_keys(doc: &mut DocumentMut, key: &str, new_section: &toml::value::Table) {
    let segments: Vec<&str> = key.split('.').collect();
    let Some((leaf, parents)) = segments.split_last() else {
        return;
    };
    let Some(table) = resolve_table_mut(doc.as_table_mut(), parents) else {
        return;
    };
    let Some(Item::Table(existing)) = table.get_mut(leaf) else {
        return;
    };
    let stale: Vec<String> = existing
        .iter()
        .map(|(name, _)| name.to_owned())
        .filter(|name| !new_section.contains_key(name))
        .collect();
    for name in stale {
        existing.remove(&name);
    }
}

/// Walks (creating as needed) to the table at `segments`.
///
/// The created tables carry no decor, so an umbrella jinn creates is
/// bare rather than borrowing a comment that belonged to a different key.
fn resolve_table_mut<'t>(root: &'t mut Table, segments: &[&str]) -> Option<&'t mut Table> {
    let mut cursor = root;
    for segment in segments {
        let entry = cursor
            .entry(segment)
            .or_insert_with(|| Item::Table(Table::new()));
        cursor = entry.as_table_mut()?;
    }
    Some(cursor)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::configurable::EntryKey;

    /// A section under an umbrella, with a scalar and a nested sub-table.
    #[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
    struct WatchdogCfg {
        #[serde(default)]
        timeout_secs: u32,
        #[serde(default)]
        stall: StallCfg,
    }

    #[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
    struct StallCfg {
        #[serde(default)]
        enabled: bool,
    }

    impl Configurable for WatchdogCfg {
        const KEY: &'static str = "watchdog.stall";
    }

    /// A section whose value has a default the type supplies and a partial
    /// document must not clobber.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct WebSearchCfg {
        #[serde(default = "default_engine")]
        engine: Option<String>,
        #[serde(default)]
        max_results: u32,
    }

    // Named for its serde `default = "..."` role, which must return the
    // same shape the field declares.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "a serde default fn returns the field's declared Option shape"
    )]
    fn default_engine() -> Option<String> {
        Some("exa".to_owned())
    }

    impl Default for WebSearchCfg {
        fn default() -> Self {
            Self {
                engine: default_engine(),
                max_results: 0,
            }
        }
    }

    impl Configurable for WebSearchCfg {
        const KEY: &'static str = "provider.web_search";
    }

    /// One entry of the lifecycle list.
    #[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
    struct LifecycleEntry {
        #[serde(default)]
        name: String,
        #[serde(default)]
        setup: String,
    }

    impl ConfigList for LifecycleEntry {
        const KEY: &'static str = "session_lifecycle.script";
        const ENTRY_KEY: &'static str = "name";
    }

    /// A section whose list is nested below its own key.
    #[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
    struct AutoPruneCfg {
        #[serde(default)]
        threshold: u32,
        #[serde(default)]
        regex: RegexCfg,
    }

    #[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
    struct RegexCfg {
        #[serde(default)]
        enabled: bool,
        #[serde(default)]
        rules: Vec<RuleCfg>,
    }

    #[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
    struct RuleCfg {
        #[serde(default)]
        pattern: String,
        #[serde(default)]
        keep_last: u32,
    }

    impl Configurable for AutoPruneCfg {
        const KEY: &'static str = "context_curation.auto_prune";
        const ENTRY_KEY: Option<EntryKey> = Some(EntryKey::new("regex.rules", "pattern"));
    }

    /// One entry of the project list, the layer's list-of-tables section.
    #[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
    struct ProjectEntry {
        #[serde(default)]
        name: String,
    }

    impl ConfigList for ProjectEntry {
        const KEY: &'static str = "project.entry";
        const ENTRY_KEY: &'static str = "name";
    }

    fn doc(body: &str) -> DocumentMut {
        body.parse::<DocumentMut>().expect("test document parses")
    }

    fn layer(body: &str) -> (ConfigLayer, Arc<InMemoryConfigStorage>) {
        let storage = Arc::new(InMemoryConfigStorage::new(doc(body)));
        let layer = ConfigLayer::load(storage.clone()).expect("layer loads");
        (layer, storage)
    }

    /// Fails when `body` carries a root-level table header named `name`.
    ///
    /// The lookup that walks a list's parent path creates the tables it
    /// passes through, so a mis-split key silently grows a table at the
    /// document root. That is invisible in a value assertion and obvious in
    /// the text, which is why this checks the text.
    fn assert_no_root_table(body: &str, name: &str) {
        let header = format!("[{name}]");
        assert!(
            !body.lines().any(|line| line.trim() == header),
            "a root-level table named {name} appeared:\n{body}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn get_reads_present_table() {
        // Given a layer over a document carrying the section.
        let (layer, _storage) = layer("[watchdog.stall]\ntimeout_secs = 300\n");

        // When reading the section.
        let value = layer.get::<WatchdogCfg>().expect("section reads");

        // Then the value comes from the document.
        assert_eq!(value.timeout_secs, 300);
    }

    #[rstest::rstest]
    #[test]
    fn get_absent_section_reads_default() {
        // Given a layer over a document with no such section.
        let (layer, _storage) = layer("[other]\nkey = 1\n");

        // When reading the absent section.
        let value = layer.get::<WatchdogCfg>().expect("absence is not an error");

        // Then it reads the type's default.
        assert_eq!(value, WatchdogCfg::default());
    }

    #[rstest::rstest]
    #[test]
    fn get_partial_section_keeps_defaults() {
        // Given a document carrying only one key of a section whose type
        // supplies a default for another.
        let (layer, _storage) = layer("[provider.web_search]\nmax_results = 5\n");

        // When reading the section.
        let value = layer.get::<WebSearchCfg>().expect("section reads");

        // Then the omitted key kept the type's default.
        assert_eq!(value.max_results, 5);
        assert_eq!(
            value.engine,
            Some("exa".to_owned()),
            "the key the document omitted kept its Default"
        );
    }

    #[rstest::rstest]
    #[test]
    fn get_malformed_section_names_section() {
        // Given a section table whose key has the wrong type.
        let (layer, _storage) = layer("[watchdog.stall]\ntimeout_secs = \"soon\"\n");

        // When reading the section.
        let result = layer.get::<WatchdogCfg>();

        // Then the failure names the section.
        assert!(matches!(
            result,
            Err(ConfigSectionError::Malformed { key, .. }) if key == "watchdog.stall"
        ));
    }

    #[rstest::rstest]
    #[test]
    fn get_non_table_segment_names_section() {
        // Given a document whose umbrella segment is a scalar, not a table.
        let (layer, _storage) = layer("watchdog = 7\n");

        // When reading the section under that umbrella.
        let result = layer.get::<WatchdogCfg>();

        // Then the read fails rather than silently defaulting.
        assert!(result.is_err(), "a non-table umbrella is a shape error");
    }

    #[rstest::rstest]
    #[test]
    fn put_writes_the_section_under_its_umbrella() {
        // Given a layer over a document with no such section.
        let (layer, _storage) = layer("");

        // When writing the section.
        let value = WatchdogCfg {
            timeout_secs: 42,
            stall: StallCfg { enabled: true },
        };
        layer.put(&value).expect("section writes");

        // Then the value is readable back through the layer.
        let read = layer.get::<WatchdogCfg>().expect("section reads");
        assert_eq!(read, value);
    }

    #[rstest::rstest]
    #[test]
    fn put_preserves_comments() {
        // Given a document whose untouched sibling key carries a comment.
        let (layer, storage) =
            layer("# a note the user wrote\nother_key = 1\n[watchdog.stall]\ntimeout_secs = 1\n");

        // When writing the section.
        layer
            .put(&WatchdogCfg {
                timeout_secs: 99,
                stall: StallCfg::default(),
            })
            .expect("section writes");

        // Then the comment survives the write.
        let text = storage.text();
        assert!(
            text.contains("# a note the user wrote"),
            "comment lost:\n{text}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn put_preserves_unmodelled_tables() {
        // Given a document carrying a table this layer knows nothing about.
        let (layer, storage) = layer("[browser]\nheadless = true\n");

        // When writing an unrelated section.
        layer
            .put(&WatchdogCfg {
                timeout_secs: 7,
                stall: StallCfg::default(),
            })
            .expect("section writes");

        // Then the unmodelled table is untouched.
        let text = storage.text();
        assert!(text.contains("[browser]"), "unmodelled table lost:\n{text}");
        assert!(
            text.contains("headless = true"),
            "unmodelled key lost:\n{text}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn put_preserves_sibling_sections() {
        // Given a document carrying a section under a sibling umbrella.
        let (layer, storage) = layer("[context_curation.compaction]\nthreshold = 0.7\n");

        // When writing a different umbrella's section.
        layer
            .put(&WatchdogCfg {
                timeout_secs: 7,
                stall: StallCfg::default(),
            })
            .expect("section writes");

        // Then the sibling section keeps its value.
        let text = storage.text();
        assert!(
            text.contains("[context_curation.compaction]"),
            "sibling section lost:\n{text}"
        );
        assert!(
            text.contains("threshold = 0.7"),
            "sibling value lost:\n{text}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn repeated_put_is_byte_identical() {
        // Given a layer holding a written section.
        let (layer, storage) = layer("[watchdog.stall]\ntimeout_secs = 1\n");
        let value = WatchdogCfg {
            timeout_secs: 55,
            stall: StallCfg { enabled: true },
        };
        layer.put(&value).expect("first write");

        // When writing the same value again.
        let after_first = storage.text();
        layer.put(&value).expect("second write");

        // Then the second write changed nothing.
        assert_eq!(
            storage.text(),
            after_first,
            "a no-change write must be a no-op on the bytes"
        );
    }

    #[rstest::rstest]
    #[test]
    fn put_removes_a_key_the_section_no_longer_declares() {
        // Given a section table carrying a key the value does not declare.
        let (layer, storage) =
            layer("[watchdog.stall]\ntimeout_secs = 1\nretired_setting = true\n");

        // When writing the section without that key.
        layer
            .put(&WatchdogCfg {
                timeout_secs: 2,
                stall: StallCfg::default(),
            })
            .expect("section writes");

        // Then the undeclared key is gone rather than lingering stale.
        let text = storage.text();
        assert!(
            !text.contains("retired_setting"),
            "a dropped key must not linger:\n{text}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn put_matches_list_entries_by_key_field() {
        // Given a list with two commented entries.
        let (layer, storage) = layer(
            "[[session_lifecycle.script]]\n# the first\nname = \"alpha\"\nsetup = \"one\"\n\n[[session_lifecycle.script]]\n# the second\nname = \"beta\"\nsetup = \"two\"\n",
        );

        // When writing the list back with only the first entry's field changed.
        layer
            .put_list::<LifecycleEntry>(&[
                LifecycleEntry {
                    name: "alpha".to_owned(),
                    setup: "one-changed".to_owned(),
                },
                LifecycleEntry {
                    name: "beta".to_owned(),
                    setup: "two".to_owned(),
                },
            ])
            .expect("list writes");

        // Then both entries' comments and the untouched value survive.
        let text = storage.text();
        assert!(text.contains("# the first"), "entry comment lost:\n{text}");
        assert!(text.contains("# the second"), "entry comment lost:\n{text}");
        assert!(
            text.contains("setup = \"one-changed\""),
            "edit lost:\n{text}"
        );
        assert!(text.contains("setup = \"two\""), "sibling lost:\n{text}");
    }

    #[rstest::rstest]
    #[test]
    fn put_registers_a_nested_list_under_the_section_key() {
        // Given a document whose section holds a nested list of rules.
        let (layer, _storage) = layer(
            "[[context_curation.auto_prune.regex.rules]]\n# keep foo\npattern = \"foo\"\nkeep_last = 1\n",
        );

        // When writing the section back.
        layer
            .put(&AutoPruneCfg {
                threshold: 10,
                regex: RegexCfg {
                    enabled: true,
                    rules: vec![RuleCfg {
                        pattern: "foo".to_owned(),
                        keep_last: 8,
                    }],
                },
            })
            .expect("section writes");

        // Then the nested entry was updated in place, comment intact.
        let text = layer.document_text();
        assert!(text.contains("# keep foo"), "entry comment lost:\n{text}");
        assert!(text.contains("keep_last = 8"), "edit lost:\n{text}");
    }

    #[rstest::rstest]
    #[test]
    fn put_replaces_a_nested_inline_list_whole() {
        // Given a document whose entry holds an inline list of policy rules.
        let (layer, storage) = layer(
            "[[session_lifecycle.script]]\nname = \"alpha\"\n# the policy\nsetup = \"one\"\n",
        );

        // When writing the section.
        layer
            .put_list::<LifecycleEntry>(&[LifecycleEntry {
                name: "alpha".to_owned(),
                setup: "one".to_owned(),
            }])
            .expect("section writes");

        // Then the document still parses and reads back.
        let text = storage.text();
        let read = layer.get_list::<LifecycleEntry>().expect("section reads");
        assert_eq!(read.len(), 1, "the entry survived the rewrite:\n{text}");
    }

    #[rstest::rstest]
    #[case::no_umbrella_at_all("")]
    #[case::umbrella_with_other_keys("[attendant]\nbehaviour = \"reset\"\n")]
    #[case::umbrella_with_an_empty_list("[attendant]\nentry = []\n")]
    #[case::umbrella_with_an_inline_array("[attendant]\nentry = [{ name = \"old\" }]\n")]
    #[case::umbrella_with_the_list_in_header_form("[[attendant.entry]]\nname = \"old\"\n")]
    fn put_list_never_creates_a_root_table_named_after_its_leaf(#[case] body: &str) {
        // Given a layer over a document in each shape the umbrella might
        // already be in.
        let (layer, storage) = layer(body);

        // When a list under that umbrella is written back.
        layer
            .put_list::<ProjectEntry>(&[ProjectEntry {
                name: "alpha".to_owned(),
            }])
            .expect("list writes");

        // Then the leaf's own name never becomes a table at the root.
        assert_no_root_table(&storage.text(), "entry");
    }

    #[rstest::rstest]
    #[case::no_umbrella_at_all("")]
    #[case::umbrella_with_the_list_in_header_form("[[session_lifecycle.script]]\nname = \"old\"\n")]
    fn put_list_does_not_leak_a_differently_named_leaf(#[case] body: &str) {
        // Given a layer over a document with no umbrella, and over one
        // whose umbrella already carries the list.
        let (layer, storage) = layer(body);

        // When a list under that umbrella is written back.
        layer
            .put_list::<LifecycleEntry>(&[LifecycleEntry {
                name: "alpha".to_owned(),
                setup: "one".to_owned(),
            }])
            .expect("list writes");

        // Then this list's leaf name does not become a root table either.
        //
        // The fix is a property of the key split, not of any one leaf
        // name, so a second leaf is what shows it generalizes.
        assert_no_root_table(&storage.text(), "script");
    }

    #[rstest::rstest]
    #[test]
    fn put_list_under_an_umbrella_preserves_surrounding_document() {
        // Given a document whose list lives under an umbrella, beside a
        // sibling entry carrying a user comment.
        let (layer, storage) = layer(
            "# existing\n[[project.entry]]\nname = \"alpha\"\n\n[[tools.bash_command_policy]]\npattern = \"rm -rf\"\n",
        );

        // When the list under the umbrella is written back.
        layer
            .put_list::<ProjectEntry>(&[ProjectEntry {
                name: "alpha".to_owned(),
            }])
            .expect("list writes");

        // Then the sibling array-of-tables and its comment are untouched.
        let text = storage.text();
        assert!(text.contains("# existing"), "comment lost:\n{text}");
        assert!(
            text.contains("pattern = \"rm -rf\""),
            "sibling list lost:\n{text}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn reload_picks_up_external_edit() {
        // Given a layer whose backing document is edited behind its back.
        let (layer, storage) = layer("[watchdog.stall]\ntimeout_secs = 1\n");

        // When the document changes and the layer reloads.
        storage
            .write(&doc("[watchdog.stall]\ntimeout_secs = 123\n"))
            .expect("external write");
        layer.reload().expect("reload succeeds");

        // Then the next read sees the edit.
        let value = layer.get::<WatchdogCfg>().expect("section reads");
        assert_eq!(value.timeout_secs, 123, "reload observed the edit");
    }

    #[rstest::rstest]
    #[test]
    fn reload_leaves_the_snapshot_intact_on_a_parse_failure() {
        // Given a layer holding a good document.
        let (layer, storage) = layer("[watchdog.stall]\ntimeout_secs = 1\n");

        // When the backing store starts handing back unparseable bytes.
        storage.set_broken(true);
        let result = layer.reload();

        // Then the reload reports failure.
        assert!(result.is_err(), "a malformed document fails the reload");

        // And the layer still holds the last good snapshot.
        let value = layer.get::<WatchdogCfg>().expect("section still reads");
        assert_eq!(
            value.timeout_secs, 1,
            "a failed reload must not blank the running snapshot"
        );
    }

    #[rstest::rstest]
    #[test]
    fn reload_is_atomic_to_readers() {
        // Given a layer over a document.
        let (layer, _storage) = layer("[watchdog.stall]\ntimeout_secs = 1\n");

        // When a reader reads while a reload swaps the document.
        let reader = layer.clone();
        let observed = std::thread::spawn(move || {
            (0..200)
                .filter_map(|_| reader.get::<WatchdogCfg>().ok())
                .map(|value| value.timeout_secs)
                .collect::<Vec<_>>()
        });
        layer.reload().expect("reload succeeds");
        let values = observed.join().expect("reader thread finished");

        // Then every read saw a whole snapshot, never a partial one.
        assert!(
            values.iter().all(|timeout| *timeout == 1),
            "a reader observed a mixed snapshot: {values:?}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn validate_rejects_a_malformed_registered_section() {
        // Given a registered section whose document table is malformed.
        let (layer, _storage) = layer("[watchdog.stall]\ntimeout_secs = \"soon\"\n");
        layer.register::<WatchdogCfg>();

        // When validating.
        let result = layer.validate();

        // Then it fails naming the section.
        assert!(matches!(
            result,
            Err(ConfigSectionError::Malformed { key, .. }) if key == "watchdog.stall"
        ));
    }

    #[rstest::rstest]
    #[test]
    fn validate_accepts_an_absent_registered_section() {
        // Given a registered section the document does not carry.
        let (layer, _storage) = layer("[other]\nkey = 1\n");
        layer.register::<WatchdogCfg>();

        // When validating.
        let result = layer.validate();

        // Then absence is fine — a section need not be written to exist.
        assert!(result.is_ok(), "an absent section validates clean");
    }

    #[rstest::rstest]
    #[test]
    fn validate_skips_an_unregistered_section() {
        // Given a malformed section that nobody registered.
        let (layer, _storage) = layer("[watchdog.stall]\ntimeout_secs = \"soon\"\n");

        // When validating a registry that does not include it.
        let result = layer.validate();

        // Then validation passes — registration is what opts a section in.
        assert!(result.is_ok(), "an unregistered section is not validated");
    }

    #[rstest::rstest]
    #[test]
    fn every_handle_reads_the_same_snapshot() {
        // Given a layer and a second handle cloned from it.
        let (layer, _storage) = layer("[watchdog.stall]\ntimeout_secs = 1\n");
        let second = layer.clone();

        // When one handle writes.
        layer
            .put(&WatchdogCfg {
                timeout_secs: 88,
                stall: StallCfg::default(),
            })
            .expect("section writes");

        // Then the other handle sees it without reloading.
        let value = second.get::<WatchdogCfg>().expect("section reads");
        assert_eq!(value.timeout_secs, 88, "handles share one snapshot");
    }
}

#[cfg(test)]
mod read_tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::ConfigLayer;
    use crate::configurable::Configurable;
    use serde::{Deserialize, Serialize};
    use std::sync::Arc;

    #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
    struct Watchdog {
        #[serde(default)]
        timeout_secs: u32,
    }

    impl Configurable for Watchdog {
        const KEY: &'static str = "watchdog.stall";
    }

    fn layer(document: &str) -> ConfigLayer {
        let parsed = document.parse().expect("test TOML parses");
        ConfigLayer::load(Arc::new(crate::InMemoryConfigStorage::new(parsed))).expect("layer loads")
    }

    /// `read` is the defaulting face a render frame or a tool call wants:
    /// a value with no sensible reaction to a bad file.
    #[rstest::rstest]
    #[test]
    fn read_returns_the_section_value() {
        // Given a layer carrying the section.
        let config = layer("[watchdog.stall]\ntimeout_secs = 42\n");

        // When reading it through the defaulting face.
        let read = config.read::<Watchdog>();

        // Then it is the document's value.
        assert_eq!(read, Watchdog { timeout_secs: 42 });
    }

    /// A malformed section is a launch-time error caught by `validate`;
    /// a running frame is better served by the documented default than by
    /// a blank pane.
    #[rstest::rstest]
    #[test]
    fn read_falls_back_to_the_default_for_a_malformed_section() {
        // Given a layer whose section has a wrong-typed field.
        let config = layer("[watchdog.stall]\ntimeout_secs = \"soon\"\n");

        // When reading it through the defaulting face.
        let read = config.read::<Watchdog>();

        // Then the default comes back rather than an error.
        assert_eq!(read, Watchdog::default());
    }
}

/// The leaf segment of a [`ConfigList`]'s dotted key, as a `&'static str`.
///
/// The patcher's array-key registry takes `&'static str` path segments.
/// `T::KEY` is an associated const, so its segments are already `'static`.
fn static_leaf<T: ConfigList>() -> &'static str {
    T::KEY.rsplit('.').next().unwrap_or(T::KEY)
}
