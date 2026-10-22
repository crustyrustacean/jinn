//! The market's generic TTL disk cache: one JSON payload per file with
//! an embedded `fetched_at` stamp.
//!
//! Pure and clock-injected — `load` takes `now` and the TTL as
//! parameters, so the caching tests drive expiry with timestamps
//! instead of sleeping. Every market payload wraps [`Stamped`] via the
//! `stamped` helper, keeping the envelope shape single-sourced.
//!
//! This is *market* vocabulary only: the provider-discovery
//! `ModelCache`/`model_cache.json` pair is a different cache with a
//! different owner and must never be read or written here.

use std::time::Duration;

use error_stack::{Report, ResultExt};
use jiff::Timestamp;
use marked_path::{Absolute, MarkedPathAccess, MarkedPathBuf};
use serde::{Deserialize, Serialize};
use wherror::Error;

/// One week of seconds — the TTLs' unit, so both constants read in
/// days. `std` has no `Duration::from_hours` to express it directly.
const WEEK_SECS: u64 = 7 * 24 * 60 * 60;

/// How long the models listing serves from disk before a refetch
/// (7 days, per the locked TTL decision).
pub const MODELS_TTL: Duration = Duration::from_secs(WEEK_SECS);

/// How long a model's endpoint detail serves from disk before a refetch.
pub const ENDPOINTS_TTL: Duration = Duration::from_secs(WEEK_SECS);

/// Errors the disk cache can hit on write. Read-side failures are
/// folded into a cache miss by [`load`]: a corrupt file degrades to a
/// refetch, it never surfaces as an error.
#[derive(Debug, Error)]
#[error(debug)]
pub enum MarketCacheError {
    /// The cache file or its parent directory could not be written.
    #[error(debug)]
    WriteFailed,
    /// The payload could not be serialized.
    #[error(debug)]
    SerializeFailed,
}

/// The on-disk envelope: the payload plus when it was fetched.
///
/// No explicit bounds: serde infers `T: Serialize` for the writer and
/// `T: Deserialize<'de>` for the reader, so a load never demands
/// serialization of the payload type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stamped<T> {
    /// When the payload was fetched from the network.
    pub fetched_at: Timestamp,
    /// The cached payload.
    pub value: T,
}

/// Wraps a freshly fetched `value` and its fetch time into the
/// [`store`](store) payload.
///
/// One constructor for the envelope shape: a payload type's cache file
/// always carries exactly this stamp, whatever the payload is.
#[must_use]
pub fn stamped<T>(value: T, fetched_at: Timestamp) -> Stamped<T>
where
    T: serde::Serialize,
{
    Stamped { fetched_at, value }
}

/// Reads the cached payload at `path`, returning it only when it parses
/// and is younger than `ttl`.
///
/// Every failure mode is a `None`: absent file, unparsable file, expired
/// stamp. A corrupt cache must degrade to a refetch, never block the
/// popup for a disk bug.
#[must_use]
pub fn load<T>(path: &MarkedPathBuf<Absolute>, ttl: Duration, now: Timestamp) -> Option<T>
where
    T: serde::de::DeserializeOwned,
{
    let raw = std::fs::read(path).ok()?;
    let stamped: Stamped<T> = serde_json::from_slice(&raw).ok()?;
    // The age in whole seconds; a sub-second remainder keeps the cache
    // fresh, and a clock run backwards (negative age) is fresh too.
    let age_secs = now
        .since(stamped.fetched_at)
        .and_then(|span| span.total(jiff::Unit::Second))
        .unwrap_or(f64::INFINITY);
    let ttl_secs = ttl.as_secs_f64();
    (age_secs <= ttl_secs).then_some(stamped.value)
}

/// Reads the cached stamp at `path` without decoding the payload.
///
/// Refresh diagnostics only; not used on the serve path.
#[must_use]
pub fn loaded_at(path: &MarkedPathBuf<Absolute>) -> Option<Timestamp> {
    let raw = std::fs::read(path).ok()?;
    let stamped: Stamped<serde_json::Value> = serde_json::from_slice(&raw).ok()?;
    Some(stamped.fetched_at)
}

/// Writes the stamped payload to `path`, creating parent directories.
///
/// # Errors
///
/// Returns [`MarketCacheError::WriteFailed`] when the file (or its
/// parent directory) cannot be created, and
/// [`MarketCacheError::SerializeFailed`] when the value cannot
/// serialize — the caller logs and moves on; a failed cache write must
/// not fail a refresh that already has its data in memory.
pub fn store<T>(
    path: &MarkedPathBuf<Absolute>,
    stamped: &Stamped<T>,
) -> Result<(), Report<MarketCacheError>>
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .change_context(MarketCacheError::WriteFailed)
            .attach(format!(
                "failed to create cache dir {}",
                parent.as_path().display()
            ))?;
    }
    let raw = serde_json::to_vec_pretty(stamped)
        .change_context(MarketCacheError::SerializeFailed)
        .attach("failed to serialize the cache payload")?;
    std::fs::write(path, raw)
        .change_context(MarketCacheError::WriteFailed)
        .attach(format!("failed to write {path}"))?;
    Ok(())
}

/// Deletes the cache file at `path` if it exists.
///
/// Best-effort: a missing file is already dropped; a failed unlink on a
/// refresh is logged by the caller. Returns whether anything was
/// removed.
pub fn drop_file(path: &MarkedPathBuf<Absolute>) -> bool {
    std::fs::remove_file(path).is_ok()
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod market_cache_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unwrap_in_result,
        reason = "test code"
    )]

    use super::*;

    /// Round-trip payload: distinct field values per variant, so a swap
    /// is visible.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    enum Probe {
        First(u32),
        Second(String),
    }

    #[rstest::rstest]
    fn load_returns_a_fresh_payload() {
        // Given a stored payload stamped now.
        let dir = jinn_testutil::tempdir();
        let path = dir
            .path
            .join("probe.json")
            .expect("temp root joins a bare name");
        let now = jiff::Timestamp::now();
        store(&path, &stamped(Probe::First(7), now)).expect("store");

        // When loading it with the same clock reading and the 7-day TTL.
        let loaded = load::<Probe>(&path, MODELS_TTL, now);

        // Then the payload comes back unchanged.
        assert_eq!(loaded, Some(Probe::First(7)));
    }

    #[rstest::rstest]
    fn load_returns_none_once_the_ttl_has_passed() {
        // Given a payload stamped 8 days ago — one day past MODELS_TTL.
        let dir = jinn_testutil::tempdir();
        let path = dir
            .path
            .join("probe.json")
            .expect("temp root joins a bare name");
        let now = jiff::Timestamp::now();
        let fetched_at = now - jiff::SignedDuration::from_hours(8 * 24);
        store(
            &path,
            &stamped(Probe::Second("stale".to_owned()), fetched_at),
        )
        .expect("store");

        // When loading it with the same clock reading.
        let loaded = load::<Probe>(&path, MODELS_TTL, now);

        // Then it is treated as absent.
        assert_eq!(loaded, None);
    }

    #[rstest::rstest]
    fn load_serves_a_payload_exactly_at_the_ttl_boundary() {
        // Given a payload stamped exactly MODELS_TTL ago.
        let dir = jinn_testutil::tempdir();
        let path = dir
            .path
            .join("probe.json")
            .expect("temp root joins a bare name");
        let now = jiff::Timestamp::now();
        let fetched_at = now - jiff::SignedDuration::from_hours(7 * 24);
        store(&path, &stamped(Probe::First(9), fetched_at)).expect("store");

        // When loading it.
        let loaded = load::<Probe>(&path, MODELS_TTL, now);

        // Then it still serves — the TTL is inclusive of its boundary.
        assert_eq!(loaded, Some(Probe::First(9)));
    }

    #[rstest::rstest]
    fn load_treats_a_corrupt_file_as_a_miss() {
        // Given a cache file that is not a market cache payload.
        let dir = jinn_testutil::tempdir();
        let path = dir
            .path
            .join("probe.json")
            .expect("temp root joins a bare name");
        std::fs::write(&path, b"not json at all").expect("write junk");

        // When loading it.
        let loaded = load::<Probe>(&path, MODELS_TTL, jiff::Timestamp::now());

        // Then it degrades to a miss rather than erroring.
        assert_eq!(loaded, None);
    }

    #[rstest::rstest]
    fn load_treats_an_absent_file_as_a_miss() {
        // Given a path that was never written.
        let dir = jinn_testutil::tempdir();
        let path = dir
            .path
            .join("never.json")
            .expect("temp root joins a bare name");

        // When loading it.
        let loaded = load::<Probe>(&path, MODELS_TTL, jiff::Timestamp::now());

        // Then it is a miss.
        assert_eq!(loaded, None);
    }

    #[rstest::rstest]
    fn load_rejects_a_type_mismatch_even_within_the_ttl() {
        // Given a fresh file stamped with a different payload type.
        let dir = jinn_testutil::tempdir();
        let path = dir
            .path
            .join("probe.json")
            .expect("temp root joins a bare name");
        let now = jiff::Timestamp::now();
        store(&path, &stamped(Probe::First(1), now)).expect("store");

        // When loading it as a mismatched type (a JSON object cannot
        // deserialize into a sequence).
        let loaded = load::<Vec<u32>>(&path, MODELS_TTL, now);

        // Then the wrong-shape payload is a miss, not a panic.
        assert_eq!(loaded, None);
    }

    #[rstest::rstest]
    fn loaded_at_reads_the_stamp_back() {
        // Given a stored payload stamped at a known time.
        let dir = jinn_testutil::tempdir();
        let path = dir
            .path
            .join("probe.json")
            .expect("temp root joins a bare name");
        let now = jiff::Timestamp::now();
        store(&path, &stamped(Probe::First(3), now)).expect("store");

        // When reading just the stamp.
        let at = loaded_at(&path);

        // Then it equals the fetch time.
        assert_eq!(at, Some(now));
    }

    #[rstest::rstest]
    fn drop_file_removes_only_an_existing_file() {
        // Given a stored payload and a path that was never written.
        let dir = jinn_testutil::tempdir();
        let path = dir
            .path
            .join("probe.json")
            .expect("temp root joins a bare name");
        let absent = dir
            .path
            .join("absent.json")
            .expect("temp root joins a bare name");
        store(&path, &stamped(Probe::First(0), jiff::Timestamp::now())).expect("store");

        // When dropping both.
        let dropped = drop_file(&path);
        let dropped_absent = drop_file(&absent);

        // Then only the existing file reports removal.
        assert!(dropped);
        assert!(!dropped_absent);
        assert!(!path.exists());
    }
}
