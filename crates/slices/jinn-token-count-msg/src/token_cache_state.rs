//! Shared per-session, per-entry token-count cache.
//!
//! Entries are append-only and immutable in this codebase, so a token count
//! is a pure function of [`jinn_core_types::ChatEntryId`]. The cache is
//! keyed two-deep so that a session close can evict an entire session in
//! O(1).
//!
//! Consumers receive a clone of [`HistoryWorkerChatEntryTokenCache`] and
//! access it only via its named methods — the underlying [`DashMap`] is
//! never exposed. The token-count slice's eviction actor holds a separate
//! clone and removes a session's inner map when the session closes; the
//! session actor's accumulation gate and the prune workers hold clones for
//! their reads.
//!
//! Thread-safe via [`DashMap`]'s internal sharding. No `RwLock` needed.
//!
//! # Naming
//!
//! Named `HistoryWorkerChatEntryTokenCache` (deliberately long) to
//! distinguish it from the persisted `token_count` field on the chat
//! entry: history workers use this memoization cache for their evaluation
//! passes, while the entry field is the durable per-entry count.

use std::sync::Arc;

use dashmap::DashMap;
use jinn_core_types::chat_entry_id::ChatEntryId;
use jinn_core_types::session_id::SessionId;

use jinn_slices::SlotKey;

/// Shared per-session, per-entry token-count cache for history workers.
///
/// Construct with [`HistoryWorkerChatEntryTokenCache::new`]; clone cheaply
/// (inner state is `Arc`-shared) to hand to multiple consumers.
#[derive(Clone, Default)]
pub struct HistoryWorkerChatEntryTokenCache {
    inner: Arc<DashMap<SessionId, DashMap<ChatEntryId, u32>>>,
}

impl HistoryWorkerChatEntryTokenCache {
    /// Create an empty shared cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
        }
    }

    /// Look up a cached token count.
    ///
    /// Returns `Some(count)` if a count was previously inserted for this
    /// `(session_id, entry_id)` pair and the session has not since been
    /// evicted. Returns `None` otherwise.
    #[must_use]
    pub fn get(&self, session_id: &SessionId, entry_id: &ChatEntryId) -> Option<u32> {
        let session_map = self.inner.get(session_id)?;
        session_map.get(entry_id).map(|v| *v)
    }

    /// Insert a token count for a `(session_id, entry_id)` pair.
    ///
    /// Overwrites any existing count for the same pair. Callers should
    /// cast from the estimator's native `usize` via `tokens as u32`.
    pub fn insert(&self, session_id: SessionId, entry_id: ChatEntryId, count: u32) {
        self.inner
            .entry(session_id)
            .or_default()
            .insert(entry_id, count);
    }

    /// Look up a cached count, or compute and store one.
    ///
    /// `compute` is called at most once per `(session_id, entry_id)` pair
    /// across the lifetime of the session's inner map. After eviction of
    /// the session via [`HistoryWorkerChatEntryTokenCache::remove_session`],
    /// a subsequent call for the same pair will re-invoke `compute`.
    ///
    /// Note: `compute` runs under a [`DashMap`] shard guard briefly. For
    /// our use case (cache hit rate → 100% after first snapshot, and
    /// tiktoken counting is microseconds) this is acceptable. If it ever
    /// becomes a hotspot, switch to a probe-then-insert pattern.
    pub fn get_or_insert_with<F>(
        &self,
        session_id: &SessionId,
        entry_id: &ChatEntryId,
        compute: F,
    ) -> u32
    where
        F: FnOnce() -> u32,
    {
        let session_id = session_id.clone();
        let entry_id = entry_id.clone();
        // outer entry() holds the outer shard guard briefly;
        // or_insert_with builds the inner DashMap only on first call.
        let session_map = self.inner.entry(session_id).or_default();

        // inner entry() holds the inner shard guard briefly;
        // or_insert_with runs the closure only on first call for this key.
        *session_map.entry(entry_id).or_insert_with(compute)
    }

    /// Evict all cached counts for a session.
    ///
    /// Called by the token-count slice's eviction actor on session close.
    /// Safe to call for a session that has no cached entries (no-op).
    /// Safe to call concurrently with `get` / `insert` / `get_or_insert_with`
    /// — [`DashMap`] operations are atomic per shard.
    pub fn remove_session(&self, session_id: &SessionId) {
        self.inner.remove(session_id);
    }
}

/// The slot key the token-count slice's cache cell lives under.
#[must_use]
pub fn token_cache_slot() -> SlotKey {
    SlotKey::builtin("token-count", "cache")
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unreachable,
        clippy::string_slice,
        clippy::uninlined_format_args,
        reason = "test code"
    )]
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    /// Deterministic session id for tests (SessionId is a Uuid newtype).
    fn test_session_id(n: u8) -> SessionId {
        serde_json::from_str(&format!("\"10000000-0000-0000-0000-{n:012x}\""))
            .expect("valid SessionId JSON")
    }

    /// Deterministic entry id for tests (ChatEntryId is a Uuid newtype).
    fn test_entry_id(n: u8) -> ChatEntryId {
        serde_json::from_str(&format!("\"00000000-0000-0000-0000-{n:012x}\""))
            .expect("valid ChatEntryId JSON")
    }

    #[rstest::rstest]
    #[test]
    fn new_cache_returns_none_for_get() {
        // Given a fresh cache and an unknown session/entry pair.
        let cache = HistoryWorkerChatEntryTokenCache::new();
        let s = test_session_id(0);
        let e = test_entry_id(0);

        // When looking the pair up.
        // Then no count is cached.
        assert_eq!(cache.get(&s, &e), None);
    }

    #[rstest::rstest]
    #[test]
    fn default_cache_returns_none_for_get() {
        // Given a default-constructed cache and an unknown session/entry pair.
        let cache = HistoryWorkerChatEntryTokenCache::default();
        let s = test_session_id(0);
        let e = test_entry_id(0);

        // When looking the pair up.
        // Then no count is cached.
        assert_eq!(cache.get(&s, &e), None);
    }

    #[rstest::rstest]
    #[test]
    fn insert_then_get_returns_value() {
        // Given a fresh cache and a session/entry pair.
        let cache = HistoryWorkerChatEntryTokenCache::new();
        let s = test_session_id(0);
        let e = test_entry_id(0);

        // When storing a count for the pair.
        cache.insert(s.clone(), e.clone(), 42);

        // Then looking the pair up returns the stored count.
        assert_eq!(cache.get(&s, &e), Some(42));
    }

    #[rstest::rstest]
    #[test]
    fn insert_overwrites_previous_value() {
        // Given a cache already holding a count for a session/entry pair.
        let cache = HistoryWorkerChatEntryTokenCache::new();
        let s = test_session_id(0);
        let e = test_entry_id(0);
        cache.insert(s.clone(), e.clone(), 42);

        // When storing a second count for the same pair.
        cache.insert(s.clone(), e.clone(), 99);

        // Then the latest count wins.
        assert_eq!(cache.get(&s, &e), Some(99));
    }

    #[rstest::rstest]
    #[test]
    fn get_or_insert_with_invokes_closure_on_first_call() {
        // Given an empty cache and a call counter.
        let cache = HistoryWorkerChatEntryTokenCache::new();
        let s = test_session_id(0);
        let e = test_entry_id(0);
        let calls = Arc::new(AtomicUsize::new(0));

        // When getting or inserting with a counting closure.
        let calls_clone = calls.clone();
        let result = cache.get_or_insert_with(&s, &e, move || {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            7
        });

        // Then the computed value is returned.
        assert_eq!(result, 7);
        // And the closure ran exactly once.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[rstest::rstest]
    #[test]
    fn get_or_insert_with_does_not_reinvoke_on_second_call() {
        // Given a cache warmed once by a counting closure.
        let cache = HistoryWorkerChatEntryTokenCache::new();
        let s = test_session_id(0);
        let e = test_entry_id(0);
        let calls = Arc::new(AtomicUsize::new(0));

        // When getting or inserting a second time.
        let calls_a = calls.clone();
        let first = cache.get_or_insert_with(&s, &e, move || {
            calls_a.fetch_add(1, Ordering::SeqCst);
            10
        });
        let calls_b = calls.clone();
        let second = cache.get_or_insert_with(&s, &e, move || {
            calls_b.fetch_add(1, Ordering::SeqCst);
            999 // should not be returned
        });

        // Then both calls see the cached value.
        assert_eq!(first, 10);
        assert_eq!(second, 10);
        // And the closure was never re-invoked.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[rstest::rstest]
    #[test]
    fn get_or_insert_with_distinguishes_entries_within_session() {
        // Given one session with two distinct entries.
        let cache = HistoryWorkerChatEntryTokenCache::new();
        let s = test_session_id(0);
        let e1 = test_entry_id(1);
        let e2 = test_entry_id(2);

        // When computing a count for each entry.
        let v1 = cache.get_or_insert_with(&s, &e1, || 100);
        let v2 = cache.get_or_insert_with(&s, &e2, || 200);

        // Then each entry keeps its own count.
        assert_eq!(v1, 100);
        assert_eq!(v2, 200);
        assert_eq!(cache.get(&s, &e1), Some(100));
        assert_eq!(cache.get(&s, &e2), Some(200));
    }

    #[rstest::rstest]
    #[test]
    fn get_or_insert_with_distinguishes_sessions() {
        // ChatEntryId uniqueness is global, so this is a defensive test —
        // the same ChatEntryId in two sessions must produce independent
        // counts because the outer key differs.
        // Given two sessions sharing one entry id.
        let cache = HistoryWorkerChatEntryTokenCache::new();
        let s1 = test_session_id(1);
        let s2 = test_session_id(2);
        let e = test_entry_id(0);

        // When computing a count for the entry in each session.
        let v1 = cache.get_or_insert_with(&s1, &e, || 11);
        let v2 = cache.get_or_insert_with(&s2, &e, || 22);

        // Then the two sessions hold independent counts.
        assert_eq!(v1, 11);
        assert_eq!(v2, 22);
        assert_eq!(cache.get(&s1, &e), Some(11));
        assert_eq!(cache.get(&s2, &e), Some(22));
    }

    #[rstest::rstest]
    #[test]
    fn remove_session_evicts_inner_map() {
        // Given a cache holding a computed count for a session.
        let cache = HistoryWorkerChatEntryTokenCache::new();
        let s = test_session_id(0);
        let e = test_entry_id(0);
        let calls = Arc::new(AtomicUsize::new(0));

        // When evicting the session.
        let calls_a = calls.clone();
        cache.get_or_insert_with(&s, &e, move || {
            calls_a.fetch_add(1, Ordering::SeqCst);
            5
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        cache.remove_session(&s);

        // Then the session's counts are gone.
        assert_eq!(cache.get(&s, &e), None);
    }

    #[rstest::rstest]
    #[test]
    fn get_or_insert_with_recomputes_after_session_eviction() {
        // Given a session whose cached count was evicted.
        let cache = HistoryWorkerChatEntryTokenCache::new();
        let s = test_session_id(0);
        let e = test_entry_id(0);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_a = calls.clone();
        cache.get_or_insert_with(&s, &e, move || {
            calls_a.fetch_add(1, Ordering::SeqCst);
            5
        });
        cache.remove_session(&s);

        // When getting or inserting for the evicted entry again.
        let calls_b = calls.clone();
        let v = cache.get_or_insert_with(&s, &e, move || {
            calls_b.fetch_add(1, Ordering::SeqCst);
            9
        });

        // Then the freshly computed value is returned.
        assert_eq!(v, 9);
        // And the closure has now run twice.
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[rstest::rstest]
    #[test]
    fn remove_session_is_noop_if_session_not_present() {
        // Given a cache with no entries for the session.
        let cache = HistoryWorkerChatEntryTokenCache::new();
        let s = test_session_id(0);

        // When evicting the absent session. Must not panic.
        cache.remove_session(&s);

        // Then the cache still holds nothing for it.
        assert_eq!(cache.get(&s, &test_entry_id(0)), None);
    }

    #[rstest::rstest]
    #[test]
    fn remove_session_does_not_affect_other_sessions() {
        // Given two sessions that both hold a count for the same entry.
        let cache = HistoryWorkerChatEntryTokenCache::new();
        let s1 = test_session_id(1);
        let s2 = test_session_id(2);
        let e = test_entry_id(0);
        cache.insert(s1.clone(), e.clone(), 111);
        cache.insert(s2.clone(), e.clone(), 222);

        // When evicting the first session.
        cache.remove_session(&s1);

        // Then only that session's count is gone.
        assert_eq!(cache.get(&s1, &e), None);
        assert_eq!(cache.get(&s2, &e), Some(222));
    }

    #[rstest::rstest]
    #[test]
    fn clone_shares_underlying_state() {
        // Given a cache holding a count and a clone of that cache.
        let cache_a = HistoryWorkerChatEntryTokenCache::new();
        let cache_b = cache_a.clone();
        let s = test_session_id(0);
        let e = test_entry_id(0);
        cache_a.insert(s.clone(), e.clone(), 50);

        // When reading through the clone, then evicting through the clone.
        // Then the clone sees the original's count.
        assert_eq!(cache_b.get(&s, &e), Some(50));
        // And the original sees the clone's eviction — shared state.
        cache_b.remove_session(&s);
        assert_eq!(cache_a.get(&s, &e), None);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn concurrent_get_or_insert_with_invokes_closure_once() {
        // N tasks all race on the same (session_id, entry_id) pair. The
        // closure increments an AtomicUsize and sleeps briefly to widen the
        // race window. Assertion: closure ran exactly once across all
        // tasks, and every task observed the same value.
        // Given N tasks racing on the same session/entry pair.
        const N: usize = 32;
        let cache = Arc::new(HistoryWorkerChatEntryTokenCache::new());
        let s = Arc::new(test_session_id(0));
        let e = Arc::new(test_entry_id(0));
        let calls = Arc::new(AtomicUsize::new(0));

        // When all tasks get-or-insert concurrently and are awaited.
        let mut handles = Vec::with_capacity(N);
        for _ in 0..N {
            let cache = Arc::clone(&cache);
            let s = Arc::clone(&s);
            let e = Arc::clone(&e);
            let calls = Arc::clone(&calls);
            handles.push(tokio::spawn(async move {
                cache.get_or_insert_with(&s, &e, move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    // Widen the race window so concurrent callers actually
                    // contend on the shard guard.
                    std::thread::sleep(Duration::from_millis(5));
                    777
                })
            }));
        }

        let mut results = Vec::with_capacity(N);
        for h in handles {
            results.push(h.await.expect("task did not panic"));
        }

        // Then the closure ran exactly once.
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "closure must run exactly once"
        );
        // And every task observed the same value.
        assert!(
            results.iter().all(|&v| v == 777),
            "all tasks must observe 777"
        );
    }
}
