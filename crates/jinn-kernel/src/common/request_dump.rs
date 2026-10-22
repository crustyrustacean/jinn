//! Request dump service — writes every outgoing provider request to a JSON file.
//!
//! When a dump directory is configured, each provider generation send writes the
//! complete assembled request payload verbatim to a separate pretty-printed JSON
//! file, named by a process-lifetime monotonic counter. This is a debugging tool
//! for verifying cache-hit stability (turn-to-turn context diffs) and inspecting
//! the exact assembled system message.
//!
//! Off by default (`dir = None`): `dump` is a no-op with effectively zero overhead.

use marked_path::{Absolute, MarkedPathAccess, MarkedPathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

/// Captures the full outgoing request payload to a directory of JSON files,
/// one per provider send. No-op when no directory is configured.
///
/// Holds an optional dump directory and a shared monotonic counter so cloned
/// services (e.g. one per actor via `Services::clone`) produce a single
/// collision-free global sequence.
#[derive(Debug, Clone)]
pub struct RequestDumpService {
    /// Destination directory. `None` disables dumping entirely.
    dir: Option<MarkedPathBuf<Absolute>>,
    /// Shared process-lifetime counter for unique zero-padded filenames.
    counter: Arc<AtomicU64>,
}

impl Default for RequestDumpService {
    fn default() -> Self {
        Self {
            dir: None,
            counter: Arc::new(AtomicU64::new(1)),
        }
    }
}

impl RequestDumpService {
    /// Creates a service that dumps to `dir`, or a disabled service when `None`.
    #[must_use]
    pub fn new(dir: Option<MarkedPathBuf<Absolute>>) -> Self {
        Self {
            dir,
            counter: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Serializes `record` to pretty JSON and writes `<dir>/<NNNNNN>.json`.
    ///
    /// No-op when no directory is configured. Write and serialization errors
    /// are logged at `warn!` and swallowed — dumping must never break a turn.
    pub fn dump<T: serde::Serialize>(&self, record: &T) {
        let Some(dir) = &self.dir else { return };
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        // A zero-padded bare filename cannot fail to join an absolute dir,
        // but a failure here degrades to the same warn-and-continue the
        // write path uses — dumping must never break a turn.
        let Ok(path) = dir.join(format!("{n:06}.json")) else {
            tracing::warn!(dir = %dir, "failed to build request dump filename");
            return;
        };
        match serde_json::to_string_pretty(record) {
            Ok(json) => {
                if let Err(e) = std::fs::create_dir_all(dir.as_path())
                    .and_then(|()| std::fs::write(path.as_path(), json))
                {
                    tracing::warn!(path = %path, err = %e, "failed to write request dump");
                }
            }
            Err(e) => tracing::warn!(err = %e, "failed to serialize request dump"),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use marked_path::MarkedPathAccess;

    /// A minimal serializable record used across the dump tests.
    #[derive(serde::Serialize)]
    struct Sample {
        msg: &'static str,
    }

    fn sample() -> Sample {
        Sample { msg: "hello" }
    }

    #[rstest::rstest]
    #[test]
    fn dump_is_noop_when_dir_is_none() {
        // Given a disabled service (dir = None) and an empty temp dir.
        let svc = RequestDumpService::default();
        let tmp = jinn_testutil::tempdir();

        // When dumping a record.
        svc.dump(&sample());

        // Then no file is written.
        let entries = std::fs::read_dir(&tmp.path).expect("read dir");
        assert_eq!(
            entries.count(),
            0,
            "no files should be written when dir is None"
        );
    }

    #[rstest::rstest]
    #[test]
    fn dump_writes_pretty_json_at_first_counter_value() {
        // Given a service pointing at a temp dir.
        let tmp = jinn_testutil::tempdir();
        let svc = RequestDumpService::new(Some(tmp.path.clone()));
        let record = sample();

        // When dumping once.
        svc.dump(&record);

        // Then 000001.json exists and equals the pretty-serialized record.
        let path = tmp
            .path
            .join("000001.json")
            .expect("temp root joins a bare name");
        let written = std::fs::read_to_string(&path).expect("read dump file");
        assert_eq!(
            written,
            serde_json::to_string_pretty(&record).expect("serialize")
        );
    }

    #[rstest::rstest]
    #[test]
    fn dump_increments_counter_across_calls() {
        // Given a service pointing at a temp dir.
        let tmp = jinn_testutil::tempdir();
        let svc = RequestDumpService::new(Some(tmp.path.clone()));

        // When dumping twice.
        svc.dump(&sample());
        svc.dump(&sample());

        // Then both 000001.json and 000002.json exist.
        assert!(
            tmp.path
                .join("000001.json")
                .expect("temp root joins a bare name")
                .exists()
        );
        assert!(
            tmp.path
                .join("000002.json")
                .expect("temp root joins a bare name")
                .exists()
        );
        assert!(
            !tmp.path
                .join("000003.json")
                .expect("temp root joins a bare name")
                .exists()
        );
    }

    #[rstest::rstest]
    #[test]
    fn dump_produces_unique_filenames_across_clones() {
        // Given a service and two clones of it (one source counter).
        let tmp = jinn_testutil::tempdir();
        let svc = RequestDumpService::new(Some(tmp.path.clone()));
        let svc_b = svc.clone();

        // When each clone dumps.
        svc.dump(&sample());
        svc_b.dump(&sample());

        // Then both filenames are unique and no collision occurred.
        let a = tmp
            .path
            .join("000001.json")
            .expect("temp root joins a bare name");
        let b = tmp
            .path
            .join("000002.json")
            .expect("temp root joins a bare name");
        assert!(a.exists(), "first clone wrote 000001.json");
        assert!(b.exists(), "second clone wrote 000002.json");
    }
}
