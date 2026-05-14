//! JSONL-backed session store implementation.
//!
//! Reads and writes `sessions.jsonl` in a platform-appropriate data directory.
//! Each line is a full [`ChatSessionState`] JSON snapshot. Multiple snapshots
//! for the same session ID may exist; [`SessionStore::load_summaries`] returns
//! the byte offset of the latest one.
//!
//! Append-only writes keep saves fast and crash-safe. Compaction rewrites the
//! file keeping only the latest snapshot per session.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead as _, BufReader, Seek as _, SeekFrom, Write as _};
use std::path::PathBuf;

use error_stack::{Report, ResultExt as _};

use crate::common::app_info::APP_NAME;
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::session_summary::SessionSummary;
use crate::protocol::SessionId;

use super::{SessionStore, SessionStoreError};

/// JSONL file name.
pub const FILE_NAME: &str = "sessions.jsonl";

/// Line count threshold that triggers compaction.
const COMPACTION_THRESHOLD: usize = 500;

/// JSONL-backed session store.
///
/// Reads and writes `sessions.jsonl` in a platform-appropriate data directory.
/// Each line is a full [`ChatSessionState`] JSON snapshot. Multiple snapshots
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
            .join(APP_NAME);
        Self { dir }
    }

    /// Creates a store at an explicit directory (for testing).
    #[must_use]
    pub fn new_in(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Returns the full path to the JSONL file.
    pub(super) fn file_path(&self) -> PathBuf {
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

    fn save(&self, session: &ChatSessionState) -> Result<(), Report<SessionStoreError>> {
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
    ) -> Result<Option<ChatSessionState>, Report<SessionStoreError>> {
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

        match serde_json::from_str::<ChatSessionState>(trimmed) {
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
        let mut latest: HashMap<SessionId, ChatSessionState> = HashMap::new();
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

            if let Ok(session) = serde_json::from_str::<ChatSessionState>(trimmed) {
                latest.insert(session.session_id().clone(), session);
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
