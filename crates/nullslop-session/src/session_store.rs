//! Session store abstraction and JSONL implementation.
//!
//! Defines [`SessionStore`] as the trait for session persistence and
//! [`JsonlSessionStore`] as the append-only JSONL file backend. Startup scans
//! lightweight [`SessionSummary`](crate::SessionSummary) entries with byte
//! offsets; full [`PersistedSession`](crate::PersistedSession) data loads on
//! demand via seek.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead as _, BufReader, Seek as _, SeekFrom, Write as _};
use std::path::PathBuf;
use std::sync::Arc;

use error_stack::{Report, ResultExt as _};
use nullslop_protocol::SessionId;
use wherror::Error;

use crate::{PersistedSession, SessionSummary};

/// Directory name under the platform data directory.
const DIR_NAME: &str = "nullslop";

/// JSONL file name.
const FILE_NAME: &str = "sessions.jsonl";

/// Line count threshold that triggers compaction.
const COMPACTION_THRESHOLD: usize = 500;

/// Error type for session store operations.
#[derive(Debug, Error)]
#[error(debug)]
pub struct SessionStoreError;

/// Abstraction for session persistence.
///
/// Every external dependency must have a trait abstraction (AGENTS.md §2).
/// Filesystem I/O is an external dependency — this trait abstracts it so
/// tests can swap in-memory storage.
pub trait SessionStore: Send + Sync + 'static {
    /// Returns the storage backend name (for debugging).
    fn name(&self) -> &'static str;

    /// Append a session snapshot to the store.
    ///
    /// The implementation writes a single JSON line. Multiple snapshots for
    /// the same session ID may exist — the latest wins on load.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the write fails.
    fn save(&self, session: &PersistedSession) -> Result<(), Report<SessionStoreError>>;

    /// Scan all lines and return lightweight summaries with byte offsets.
    ///
    /// Each entry is `(session_id, summary, byte_offset)` where `byte_offset`
    /// is the position of the **latest** line for that session in the file.
    /// Used for startup index building and on-demand full loads.
    ///
    /// Corrupted or unparseable lines are skipped gracefully.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the file cannot be opened or read.
    fn load_summaries(
        &self,
    ) -> Result<Vec<(SessionId, SessionSummary, u64)>, Report<SessionStoreError>>;

    /// Load a full session by seeking to the given byte offset.
    ///
    /// Returns `None` if the line at the offset cannot be parsed as a
    /// [`PersistedSession`].
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the seek or read fails.
    fn load_full(
        &self,
        byte_offset: u64,
    ) -> Result<Option<PersistedSession>, Report<SessionStoreError>>;

    /// Rewrite the store, keeping only the latest snapshot per session.
    ///
    /// Call when the file grows beyond a threshold (e.g., 500 lines).
    /// After compaction, byte offsets from previous `load_summaries` calls
    /// are invalidated — callers must re-scan.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the rewrite fails.
    fn compact(&self) -> Result<(), Report<SessionStoreError>>;
}

/// Service wrapper for session storage.
///
/// Wraps `Arc<dyn SessionStore>` for shared ownership across the application.
/// Follows the service wrapper pattern from the project style guide.
#[derive(Debug, Clone)]
pub struct SessionStoreService {
    /// The underlying session store implementation.
    svc: Arc<dyn SessionStore>,
}

impl SessionStoreService {
    /// Creates a new session store service.
    #[must_use]
    pub fn new(store: Arc<dyn SessionStore>) -> Self {
        Self { svc: store }
    }

    /// Append a session snapshot to the store.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the write fails.
    pub fn save(&self, session: &PersistedSession) -> Result<(), Report<SessionStoreError>> {
        self.svc.save(session)
    }

    /// Scan all lines and return lightweight summaries with byte offsets.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the file cannot be opened or read.
    pub fn load_summaries(
        &self,
    ) -> Result<Vec<(SessionId, SessionSummary, u64)>, Report<SessionStoreError>> {
        self.svc.load_summaries()
    }

    /// Load a full session by seeking to the given byte offset.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the seek or read fails.
    pub fn load_full(
        &self,
        byte_offset: u64,
    ) -> Result<Option<PersistedSession>, Report<SessionStoreError>> {
        self.svc.load_full(byte_offset)
    }

    /// Rewrite the store, keeping only the latest snapshot per session.
    ///
    /// # Errors
    ///
    /// Returns [`SessionStoreError`] if the rewrite fails.
    pub fn compact(&self) -> Result<(), Report<SessionStoreError>> {
        self.svc.compact()
    }
}

impl std::fmt::Debug for dyn SessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionStore")
            .field("name", &self.name())
            .finish()
    }
}

/// JSONL-backed session store.
///
/// Reads and writes `sessions.jsonl` in a platform-appropriate data directory.
/// Each line is a full [`PersistedSession`] JSON snapshot. Multiple snapshots
/// for the same session ID may exist; [`SessionStore::load_summaries`] returns
/// the byte offset of the latest one.
///
/// Append-only writes keep saves fast and crash-safe. Compaction rewrites the
/// file keeping only the latest snapshot per session.
pub struct JsonlSessionStore {
    /// Directory containing `sessions.jsonl`.
    dir: PathBuf,
}

impl Default for JsonlSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl JsonlSessionStore {
    /// Creates a store at the platform data directory.
    ///
    /// Uses `dirs::data_dir()` → `nullslop/sessions.jsonl` on Linux.
    /// Does not create the directory until the first [`SessionStore::save`].
    ///
    /// # Panics
    ///
    /// Panics if the platform data directory cannot be determined.
    #[expect(
        clippy::expect_used,
        reason = "platform data dir is always available on supported targets"
    )]
    #[must_use]
    pub fn new() -> Self {
        let dir = dirs::data_dir()
            .expect("platform data directory should be available")
            .join(DIR_NAME);
        Self { dir }
    }

    /// Creates a store at an explicit directory (for testing).
    #[must_use]
    pub fn new_in(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Returns the full path to the JSONL file.
    fn file_path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    /// Ensures the directory exists, creating it if needed.
    fn ensure_dir(&self) -> Result<(), Report<SessionStoreError>> {
        if !self.dir.exists() {
            fs::create_dir_all(&self.dir)
                .change_context(SessionStoreError)
                .attach("failed to create session directory")?;
        }
        Ok(())
    }
}

impl SessionStore for JsonlSessionStore {
    fn name(&self) -> &'static str {
        "jsonl"
    }

    fn save(&self, session: &PersistedSession) -> Result<(), Report<SessionStoreError>> {
        self.ensure_dir()?;

        let line = serde_json::to_string(session)
            .change_context(SessionStoreError)
            .attach("failed to serialize session")?;

        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(self.file_path())
            .change_context(SessionStoreError)
            .attach("failed to open sessions file for append")?;

        writeln!(file, "{line}")
            .change_context(SessionStoreError)
            .attach("failed to write session line")?;

        Ok(())
    }

    fn load_summaries(
        &self,
    ) -> Result<Vec<(SessionId, SessionSummary, u64)>, Report<SessionStoreError>> {
        let path = self.file_path();
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&path)
            .change_context(SessionStoreError)
            .attach("failed to open sessions file for reading")?;

        let mut reader = BufReader::new(file);
        let mut latest: HashMap<SessionId, (SessionSummary, u64)> = HashMap::new();
        let mut offset: u64 = 0;

        loop {
            let mut line = String::new();
            let bytes_read = reader
                .read_line(&mut line)
                .change_context(SessionStoreError)
                .attach("failed to read line")?;

            if bytes_read == 0 {
                break;
            }

            let line_start = offset;
            offset += bytes_read as u64;

            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                continue;
            }

            if let Ok(summary) = serde_json::from_str::<SessionSummary>(trimmed) {
                latest.insert(summary.session_id.clone(), (summary, line_start));
            }
            // Corrupted lines are silently skipped.
        }

        Ok(latest
            .into_iter()
            .map(|(id, (summary, offset))| (id, summary, offset))
            .collect())
    }

    fn load_full(
        &self,
        byte_offset: u64,
    ) -> Result<Option<PersistedSession>, Report<SessionStoreError>> {
        let path = self.file_path();
        if !path.exists() {
            return Ok(None);
        }

        let mut file = File::open(&path)
            .change_context(SessionStoreError)
            .attach("failed to open sessions file for full load")?;

        file.seek(SeekFrom::Start(byte_offset))
            .change_context(SessionStoreError)
            .attach("failed to seek to byte offset")?;

        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let bytes_read = reader
            .read_line(&mut line)
            .change_context(SessionStoreError)
            .attach("failed to read line at offset")?;

        if bytes_read == 0 {
            return Ok(None);
        }

        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            return Ok(None);
        }

        match serde_json::from_str::<PersistedSession>(trimmed) {
            Ok(session) => Ok(Some(session)),
            Err(_) => Ok(None),
        }
    }

    fn compact(&self) -> Result<(), Report<SessionStoreError>> {
        let path = self.file_path();
        if !path.exists() {
            return Ok(());
        }

        // Read all lines, keeping only the latest per session.
        let file = File::open(&path)
            .change_context(SessionStoreError)
            .attach("failed to open sessions file for compaction")?;

        let reader = BufReader::new(file);
        let mut latest: HashMap<SessionId, PersistedSession> = HashMap::new();
        let mut total_lines: usize = 0;

        for line_result in reader.lines() {
            let line: String = line_result
                .change_context(SessionStoreError)
                .attach("failed to read line during compaction")?;

            total_lines += 1;

            let trimmed = line.trim_end();
            if trimmed.is_empty() {
                continue;
            }

            if let Ok(session) = serde_json::from_str::<PersistedSession>(trimmed) {
                latest.insert(session.session_id.clone(), session);
            }
            // Corrupted lines are silently skipped.
        }

        // Skip compaction if below threshold.
        if total_lines <= COMPACTION_THRESHOLD {
            return Ok(());
        }

        // Write compacted file to a temp file in the same directory, then rename.
        let tmp_path = self.dir.join(format!(".{FILE_NAME}.tmp"));

        let mut tmp_file = File::create(&tmp_path)
            .change_context(SessionStoreError)
            .attach("failed to create temp file for compaction")?;

        for session in latest.values() {
            let line = serde_json::to_string(session)
                .change_context(SessionStoreError)
                .attach("failed to serialize session during compaction")?;

            writeln!(tmp_file, "{line}")
                .change_context(SessionStoreError)
                .attach("failed to write session during compaction")?;
        }

        tmp_file
            .sync_all()
            .change_context(SessionStoreError)
            .attach("failed to sync temp file")?;

        fs::rename(&tmp_path, &path)
            .change_context(SessionStoreError)
            .attach("failed to rename compacted file")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use nullslop_protocol::{ChatEntry, PromptStrategyId, SessionId};
    use tempfile::TempDir;

    use super::*;
    use crate::PersistedSession;

    /// Creates a minimal `PersistedSession` for testing.
    fn make_session(id: &SessionId, title: &str) -> PersistedSession {
        PersistedSession {
            session_id: id.clone(),
            title: title.to_owned(),
            updated_at: Timestamp::now(),
            history: vec![ChatEntry::user("hello")],
            active_strategy: PromptStrategyId::passthrough(),
            blobs: std::collections::HashMap::new(),
        }
    }

    // --- Test 1: Save + load round-trip ---

    #[test]
    fn save_creates_summary() {
        // Given a JsonlSessionStore in a temp directory.
        let dir = TempDir::new().expect("temp dir");
        let store = JsonlSessionStore::new_in(dir.path().to_path_buf());
        let session_id = SessionId::new();
        let session = make_session(&session_id, "Test Session");

        // When saving and loading summaries.
        store.save(&session).expect("save");
        let summaries = store.load_summaries().expect("load_summaries");

        // Then one summary is returned.
        assert_eq!(summaries.len(), 1);
        let (id, summary, _offset) = &summaries[0];
        assert_eq!(id, &session_id);
        assert_eq!(summary.title, "Test Session");
    }

    #[test]
    fn load_full_restores_data() {
        // Given a JsonlSessionStore in a temp directory.
        let dir = TempDir::new().expect("temp dir");
        let store = JsonlSessionStore::new_in(dir.path().to_path_buf());
        let session_id = SessionId::new();
        let session = make_session(&session_id, "Test Session");

        // When saving and loading full at the summary offset.
        store.save(&session).expect("save");
        let summaries = store.load_summaries().expect("load_summaries");
        let offset = summaries[0].2;

        // Then load_full returns the complete session.
        let full = store
            .load_full(offset)
            .expect("load_full")
            .expect("should have a session");
        assert_eq!(full.session_id, session_id);
        assert_eq!(full.title, "Test Session");
        assert_eq!(full.history.len(), 1);
    }

    // --- Test 2: Multiple sessions, latest wins ---

    #[test]
    fn summaries_returns_correct_count() {
        // Given a store with 3 saves: session A (v1), session B (v1), session A (v2).
        let dir = TempDir::new().expect("temp dir");
        let store = JsonlSessionStore::new_in(dir.path().to_path_buf());

        let id_a = SessionId::new();
        let id_b = SessionId::new();

        store.save(&make_session(&id_a, "A v1")).expect("save A v1");
        store.save(&make_session(&id_b, "B v1")).expect("save B v1");
        store.save(&make_session(&id_a, "A v2")).expect("save A v2");

        // When loading summaries.
        let summaries = store.load_summaries().expect("load_summaries");

        // Then 2 summaries are returned.
        assert_eq!(summaries.len(), 2);
    }

    #[test]
    fn summaries_returns_latest_versions() {
        // Given a store with 3 saves: session A (v1), session B (v1), session A (v2).
        let dir = TempDir::new().expect("temp dir");
        let store = JsonlSessionStore::new_in(dir.path().to_path_buf());

        let id_a = SessionId::new();
        let id_b = SessionId::new();

        store.save(&make_session(&id_a, "A v1")).expect("save A v1");
        store.save(&make_session(&id_b, "B v1")).expect("save B v1");
        store.save(&make_session(&id_a, "A v2")).expect("save A v2");

        // When loading summaries.
        let summaries = store.load_summaries().expect("load_summaries");

        // Then A is v2 and B is v1.
        let entry_a = summaries
            .iter()
            .find(|(id, _, _)| id == &id_a)
            .expect("session A should exist");
        assert_eq!(entry_a.1.title, "A v2");

        let entry_b = summaries
            .iter()
            .find(|(id, _, _)| id == &id_b)
            .expect("session B should exist");
        assert_eq!(entry_b.1.title, "B v1");
    }

    #[test]
    fn byte_offset_points_to_latest() {
        // Given a store with 3 saves: session A (v1), session B (v1), session A (v2).
        let dir = TempDir::new().expect("temp dir");
        let store = JsonlSessionStore::new_in(dir.path().to_path_buf());

        let id_a = SessionId::new();
        let id_b = SessionId::new();

        store.save(&make_session(&id_a, "A v1")).expect("save A v1");
        store.save(&make_session(&id_b, "B v1")).expect("save B v1");
        store.save(&make_session(&id_a, "A v2")).expect("save A v2");

        // When loading summaries and seeking to A's offset.
        let summaries = store.load_summaries().expect("load_summaries");

        let entry_a = summaries
            .iter()
            .find(|(id, _, _)| id == &id_a)
            .expect("session A should exist");

        // Then session A's byte offset points to the second save (v2).
        let full_a = store
            .load_full(entry_a.2)
            .expect("load_full")
            .expect("should have session A");
        assert_eq!(full_a.title, "A v2");
    }

    // --- Test 3: Compaction removes stale snapshots ---

    #[test]
    fn compact_preserves_sessions() {
        // Given a store with 600+ lines (multiple saves of 2 sessions).
        let dir = TempDir::new().expect("temp dir");
        let store = JsonlSessionStore::new_in(dir.path().to_path_buf());

        let id_a = SessionId::new();
        let id_b = SessionId::new();

        for i in 0..300 {
            store
                .save(&make_session(&id_a, &format!("A iter {i}")))
                .expect("save A");
            store
                .save(&make_session(&id_b, &format!("B iter {i}")))
                .expect("save B");
        }
        // 600 lines total, above the 500 threshold.

        // When compacting.
        store.compact().expect("compact");

        // Then summaries still return the same sessions.
        let summaries = store.load_summaries().expect("load_summaries");
        assert_eq!(summaries.len(), 2);
    }

    #[test]
    fn compact_reduces_file_size() {
        // Given a store with 600+ lines (multiple saves of 2 sessions).
        let dir = TempDir::new().expect("temp dir");
        let store = JsonlSessionStore::new_in(dir.path().to_path_buf());

        let id_a = SessionId::new();
        let id_b = SessionId::new();

        for i in 0..300 {
            store
                .save(&make_session(&id_a, &format!("A iter {i}")))
                .expect("save A");
            store
                .save(&make_session(&id_b, &format!("B iter {i}")))
                .expect("save B");
        }
        // 600 lines total, above the 500 threshold.

        // When compacting.
        store.compact().expect("compact");

        // Then the file now has only 2 lines (one per session).
        let content = fs::read_to_string(store.file_path()).expect("read file");
        let line_count = content.lines().filter(|l| !l.is_empty()).count();
        assert_eq!(line_count, 2);
    }

    // --- Test 4: Compaction is no-op below threshold ---

    #[test]
    fn compact_is_noop_below_threshold() {
        // Given a store with 10 lines (below threshold).
        let dir = TempDir::new().expect("temp dir");
        let store = JsonlSessionStore::new_in(dir.path().to_path_buf());

        let id = SessionId::new();
        for i in 0..10 {
            store
                .save(&make_session(&id, &format!("iter {i}")))
                .expect("save");
        }

        let content_before = fs::read_to_string(store.file_path()).expect("read file before");

        // When compacting.
        store.compact().expect("compact");

        // Then the file is unchanged.
        let content_after = fs::read_to_string(store.file_path()).expect("read file after");
        assert_eq!(content_before, content_after);
    }

    // --- Test 5: Corrupted lines skipped gracefully ---

    #[test]
    fn load_summaries_skips_corrupted_lines() {
        // Given a JSONL file with a valid line, a corrupted line, and another valid line.
        let dir = TempDir::new().expect("temp dir");
        let store = JsonlSessionStore::new_in(dir.path().to_path_buf());

        let id_a = SessionId::new();
        let id_b = SessionId::new();

        store.save(&make_session(&id_a, "Valid A")).expect("save A");

        // Write a corrupted line directly.
        {
            let mut file = OpenOptions::new()
                .append(true)
                .open(store.file_path())
                .expect("open");
            writeln!(file, "this is not valid json").expect("write");
        }

        store.save(&make_session(&id_b, "Valid B")).expect("save B");

        // When loading summaries.
        let summaries = store.load_summaries().expect("load_summaries");

        // Then 2 valid summaries are returned (the corrupted line is skipped).
        assert_eq!(summaries.len(), 2);
        let titles: Vec<&str> = summaries.iter().map(|(_, s, _)| s.title.as_str()).collect();
        assert!(titles.contains(&"Valid A"));
        assert!(titles.contains(&"Valid B"));
    }

    // --- Test 6: Load full from corrupted offset returns None ---

    #[test]
    fn load_full_returns_none_for_corrupted_line() {
        // Given a JSONL file with a corrupted line at a known offset.
        let dir = TempDir::new().expect("temp dir");
        let store = JsonlSessionStore::new_in(dir.path().to_path_buf());

        // Write a corrupted line and capture its offset.
        let corrupted_offset = {
            let mut file = File::create(store.file_path()).expect("create");
            let offset = file.stream_position().expect("position");
            writeln!(file, "not json").expect("write");
            offset
        };

        // When loading full at the corrupted offset.
        let result = store.load_full(corrupted_offset).expect("load_full");

        // Then None is returned (graceful degradation).
        assert!(result.is_none());
    }

    // --- Test 7: Load from empty store returns empty ---

    #[test]
    fn load_summaries_returns_empty_when_no_file() {
        // Given a JsonlSessionStore in a temp directory with no file.
        let dir = TempDir::new().expect("temp dir");
        let store = JsonlSessionStore::new_in(dir.path().to_path_buf());

        // When loading summaries.
        let summaries = store.load_summaries().expect("load_summaries");

        // Then an empty vec is returned.
        assert!(summaries.is_empty());
    }

    // --- Test 8: Save creates directory if missing ---

    #[test]
    fn save_creates_directory() {
        // Given a JsonlSessionStore pointed at a non-existent directory.
        let dir = TempDir::new().expect("temp dir");
        let nested = dir.path().join("does").join("not").join("exist");
        let store = JsonlSessionStore::new_in(nested.clone());
        let session = make_session(&SessionId::new(), "Mkdir Test");

        // When saving.
        store.save(&session).expect("save");

        // Then the directory is created.
        assert!(nested.exists());
    }

    #[test]
    fn save_creates_file() {
        // Given a JsonlSessionStore pointed at a non-existent directory.
        let dir = TempDir::new().expect("temp dir");
        let nested = dir.path().join("does").join("not").join("exist");
        let store = JsonlSessionStore::new_in(nested.clone());
        let session = make_session(&SessionId::new(), "Mkdir Test");

        // When saving.
        store.save(&session).expect("save");

        // Then the file is created.
        assert!(nested.join(FILE_NAME).exists());
    }

    #[test]
    fn save_returns_summary() {
        // Given a JsonlSessionStore pointed at a non-existent directory.
        let dir = TempDir::new().expect("temp dir");
        let nested = dir.path().join("does").join("not").join("exist");
        let store = JsonlSessionStore::new_in(nested.clone());
        let session = make_session(&SessionId::new(), "Mkdir Test");

        // When saving and loading summaries.
        store.save(&session).expect("save");

        // Then load_summaries returns the saved session.
        let summaries = store.load_summaries().expect("load_summaries");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].1.title, "Mkdir Test");
    }

    // --- Test 9: Load full at offset returns correct session among many ---

    #[test]
    fn load_full_at_offset_returns_correct_session_among_many() {
        // Given a store with sessions A, B, C saved in sequence.
        let dir = TempDir::new().expect("temp dir");
        let store = JsonlSessionStore::new_in(dir.path().to_path_buf());

        let id_a = SessionId::new();
        let id_b = SessionId::new();
        let id_c = SessionId::new();

        store
            .save(&make_session(&id_a, "Session A"))
            .expect("save A");
        store
            .save(&make_session(&id_b, "Session B"))
            .expect("save B");
        store
            .save(&make_session(&id_c, "Session C"))
            .expect("save C");

        let summaries = store.load_summaries().expect("load_summaries");

        // When loading full at session B's offset.
        let entry_b = summaries
            .iter()
            .find(|(id, _, _)| id == &id_b)
            .expect("session B should exist");

        let full_b = store
            .load_full(entry_b.2)
            .expect("load_full")
            .expect("should have session B");

        // Then session B's data is returned (not A or C).
        assert_eq!(full_b.session_id, id_b);
        assert_eq!(full_b.title, "Session B");
    }
}
