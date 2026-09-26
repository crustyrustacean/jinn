//! Line count cache for virtualized chat log rendering.
//!
//! Caches the wrapped line count *and rendered lines* per entry so the renderer
//! can cheaply determine which entries are visible without calling
//! `entry_to_lines()` for the entire history. On a cache hit, the pre-rendered
//! `Vec<Line>` is reused in Pass 2 - skipping both parsing and rendering.
//!
//! The cache is invalidated on content changes (streaming tokens),
//! expand/collapse toggles, content width changes (terminal resize), and
//! render-variant changes (status-derived look that alters the rendered lines
//! without touching the entry's content — e.g. a paired tool result landing
//! or a subagent's running state flipping).
//! Theme changes are handled centrally by [`FrontendCaches::invalidate_all`]
//! which calls [`EntryLineCache::clear`] directly.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ratatui::text::Line;

use jinn_core_types::{ChatEntry, ChatEntryId};

/// How many entries may hold pre-rendered lines at once.
///
/// The wrapped line count is retained for every entry regardless — it is tiny,
/// and keeping it is what makes the layout pass cheap. Only the rendered
/// `Line` payload is bounded, so memory stops growing with session length.
/// Roughly the last few thousand lines of scrollback.
pub const MAX_CACHED_RENDERED_ENTRIES: usize = 4_000;

/// Cached wrapped line count and rendered lines for a single entry.
#[derive(Debug, Clone)]
pub struct CachedEntryCount {
    /// Fingerprint of the entry's content when this count was computed.
    pub fingerprint: u64,
    /// O(1) summary of the same content, used to skip re-hashing when the
    /// entry is looked up again and is very likely unchanged.
    pub signature: u64,
    /// Whether the entry was expanded when this count was computed.
    pub is_expanded: bool,
    /// Hash of the status-derived render inputs (paired result status,
    /// streaming flag, subagent-waiting flag) at compute time.
    pub variant: u64,
    /// The wrapped line count for this entry.
    pub wrapped_count: u32,
    /// The content width this count was computed at.
    ///
    /// Per entry rather than per cache: two sessions can be measured at
    /// different widths, and a single cache-wide width would let one session's
    /// render discard every count the other had already paid for.
    pub content_width: u16,
    /// Pre-rendered lines for this entry, if available.
    ///
    /// `None` when inserted via [`EntryLineCache::insert`] (count-only).
    /// `Some` when inserted via [`EntryLineCache::insert_with_lines`].
    #[expect(
        clippy::rc_buffer,
        reason = "Arc keeps cloning a rendered entry's line buffer O(1) where a plain Vec would deep-copy every line on each cache hit"
    )]
    pub lines: Option<Arc<Vec<Line<'static>>>>,
    /// The `touch_counter` value when this entry's lines were last used.
    ///
    /// Only the painting pass bumps this, so it tracks "scrolled to", not
    /// "walked past during layout".
    pub last_touched: u64,
}

/// Result of a successful cache hit.
pub struct CacheHit {
    /// The wrapped line count for this entry.
    pub wrapped_count: u32,
    /// Pre-rendered lines for this entry, if they were cached.
    #[expect(
        clippy::rc_buffer,
        reason = "Arc keeps cloning a rendered entry's line buffer O(1) where a plain Vec would deep-copy every line on each cache hit"
    )]
    pub lines: Option<Arc<Vec<Line<'static>>>>,
}

/// The two hashes describing an entry's content at probe time.
///
/// Bundled so a miss can be stored without hashing the entry again: the
/// fingerprint may have been skipped entirely when the signature matched.
#[derive(Debug, Clone, Copy)]
pub struct ContentIdentity {
    /// The O(1) content summary.
    pub signature: u64,
    /// The full content hash.
    pub fingerprint: u64,
}

/// The outcome of a cache probe: the hit, if any, plus the content identity to
/// store when storing a fresh count.
pub struct CacheProbe {
    /// The cached count and lines, when the entry is unchanged.
    pub hit: Option<CacheHit>,
    /// Content identity observed during this probe.
    pub content: ContentIdentity,
}

/// One entry's measured wrapped line count, ready to be stored.
///
/// Carries everything the cache keys on except the rendered lines, which the
/// measurement discards. Produced by the off-thread layout worker and applied
/// in one batch by [`EntryLineCache::insert_counts`].
#[derive(Debug, Clone)]
pub struct MeasuredLineCount {
    /// The entry this count describes.
    pub id: ChatEntryId,
    /// The entry's content identity as the worker observed it.
    pub content: ContentIdentity,
    /// Whether the entry was expanded when the count was computed.
    pub is_expanded: bool,
    /// Hash of the status-derived render inputs at compute time.
    pub variant: u64,
    /// The measured wrapped line count.
    pub wrapped_count: u32,
}

/// Cache mapping entry IDs to their cached wrapped line counts and rendered lines.
///
/// Owned by [`FrontendCaches`] - populated during the render pass, used
/// to determine which entries overlap the viewport without re-rendering
/// the entire history.
///
/// # Invalidation
///
/// - **Content width change:** clears all entries.
/// - **Theme change:** cleared by [`FrontendCaches::invalidate_all`].
/// - **Streaming (content change):** detected by fingerprint mismatch → automatic miss.
/// - **Expand/collapse:** detected by `is_expanded` mismatch → automatic miss.
/// - **New entry:** no cache entry exists → automatic miss.
#[derive(Debug, Default)]
pub struct EntryLineCache {
    /// Per-entry cached counts.
    entries: HashMap<ChatEntryId, CachedEntryCount>,
    /// How many times a full content fingerprint has been computed.
    ///
    /// The whole point of the signature gate is that this stays flat across
    /// steady-state frames, so it is worth being able to observe in tests.
    /// It is an atomic so a hash can be counted while a cached entry is still
    /// borrowed, and so the cache stays `Sync` under its `RwLock`.
    fingerprint_computations: AtomicU64,
    /// Monotonic clock for LRU recency, bumped on every touch.
    touch_counter: u64,
    /// How many rendered-line payloads have been dropped by eviction.
    evictions: u64,
}

impl Clone for EntryLineCache {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            // A clone starts its own tally rather than inheriting a count
            // that says nothing about the entries it now owns.
            fingerprint_computations: AtomicU64::new(0),
            touch_counter: self.touch_counter,
            evictions: self.evictions,
        }
    }
}

impl EntryLineCache {
    /// Create a new empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up the cached wrapped line count and optional rendered lines for an entry.
    ///
    /// Returns `None` if:
    /// - No cache entry exists for this ID (new entry).
    /// - The entry's fingerprint has changed (content changed during streaming).
    /// - The entry's expanded state has changed (expand/collapse toggle).
    /// - The entry's render variant has changed (status-derived look changed).
    /// - The content width has changed (terminal resize).
    pub fn get(
        &mut self,
        entry: &ChatEntry,
        is_expanded: bool,
        variant: u64,
        content_width: u16,
    ) -> Option<CacheHit> {
        self.probe(entry, is_expanded, variant, content_width).hit
    }

    /// Look up an entry, also reporting the content fingerprint that was
    /// computed (or skipped) so the caller can store it on a miss instead of
    /// hashing the entry a second time.
    pub fn probe(
        &mut self,
        entry: &ChatEntry,
        is_expanded: bool,
        variant: u64,
        content_width: u16,
    ) -> CacheProbe {
        let signature = entry.content_signature();
        let Some(cached) = self.entries.get(&entry.id) else {
            return CacheProbe {
                hit: None,
                content: self.fresh_content(entry),
            };
        };

        // Measured at another width, so the count describes a layout nothing is
        // rendering at. Scoped to this entry: a resize only invalidates the
        // counts that were taken at the old width, and only once the width
        // actually differs from the one this entry was measured at.
        if cached.content_width != content_width {
            return CacheProbe {
                hit: None,
                content: self.fresh_content(entry),
            };
        }

        // The signature is O(1) and covers every field the fingerprint reads.
        // When it matches, the content is almost certainly unchanged, so the
        // full hash — which costs time proportional to the entry's size — is
        // skipped entirely.
        let content = if cached.signature == signature {
            ContentIdentity {
                signature,
                fingerprint: cached.fingerprint,
            }
        } else {
            ContentIdentity {
                signature,
                fingerprint: self.fingerprint_of(entry),
            }
        };

        CacheProbe {
            hit: (content.fingerprint == cached.fingerprint
                && cached.is_expanded == is_expanded
                && cached.variant == variant)
                .then(|| CacheHit {
                    wrapped_count: cached.wrapped_count,
                    lines: cached.lines.clone(),
                }),
            content,
        }
    }

    /// Compute and count a full content fingerprint.
    fn fingerprint_of(&self, entry: &ChatEntry) -> u64 {
        self.fingerprint_computations
            .fetch_add(1, Ordering::Relaxed);
        entry.content_fingerprint()
    }

    /// Fingerprint an entry with no cached counterpart to compare against.
    fn fresh_content(&self, entry: &ChatEntry) -> ContentIdentity {
        ContentIdentity {
            signature: entry.content_signature(),
            fingerprint: self.fingerprint_of(entry),
        }
    }

    /// How many full content fingerprints this cache has computed.
    #[must_use]
    pub fn fingerprint_computations(&self) -> u64 {
        self.fingerprint_computations.load(Ordering::Relaxed)
    }

    /// Mark an entry's rendered lines as recently used.
    ///
    /// Call this from the painting pass only. Layout walks every entry on every
    /// frame, so touching there would make every entry equally recent and the
    /// LRU would degenerate to insertion order.
    pub fn touch(&mut self, id: &ChatEntryId) {
        if let Some(cached) = self.entries.get_mut(id) {
            self.touch_counter = self.touch_counter.wrapping_add(1);
            cached.last_touched = self.touch_counter;
        }
    }

    /// How many rendered-line payloads eviction has dropped.
    #[must_use]
    pub fn evictions(&self) -> u64 {
        self.evictions
    }

    /// Drop rendered lines beyond [`MAX_CACHED_RENDERED_ENTRIES`], least
    /// recently used first.
    ///
    /// Only the `lines` payload is dropped: the wrapped count always survives,
    /// so eviction never costs a re-layout.
    pub fn evict_if_needed(&mut self) {
        while self.rendered_entry_count() > MAX_CACHED_RENDERED_ENTRIES {
            let Some(victim) = self.least_recently_touched() else {
                break;
            };
            if let Some(cached) = self.entries.get_mut(&victim) {
                cached.lines = None;
            }
            self.evictions += 1;
        }
    }

    /// Number of entries currently holding rendered lines.
    fn rendered_entry_count(&self) -> usize {
        self.entries.values().filter(|c| c.lines.is_some()).count()
    }

    /// The id of the entry holding lines that were used longest ago.
    fn least_recently_touched(&self) -> Option<ChatEntryId> {
        self.entries
            .iter()
            .filter(|(_, c)| c.lines.is_some())
            .min_by_key(|(_, c)| c.last_touched)
            .map(|(id, _)| id.clone())
    }

    /// Store a wrapped line count for an entry (without rendered lines).
    pub fn insert(
        &mut self,
        entry: &ChatEntry,
        content: ContentIdentity,
        is_expanded: bool,
        variant: u64,
        content_width: u16,
        wrapped_count: u32,
    ) {
        self.entries.insert(
            entry.id.clone(),
            CachedEntryCount {
                content_width,
                fingerprint: content.fingerprint,
                signature: content.signature,
                is_expanded,
                variant,
                wrapped_count,
                lines: None,
                last_touched: self.touch_counter,
            },
        );
    }

    /// Store a wrapped line count and rendered lines for an entry.
    pub fn insert_with_lines(
        &mut self,
        entry: &ChatEntry,
        content: ContentIdentity,
        is_expanded: bool,
        variant: u64,
        content_width: u16,
        wrapped_count: u32,
        lines: Arc<Vec<Line<'static>>>,
    ) {
        self.entries.insert(
            entry.id.clone(),
            CachedEntryCount {
                content_width,
                fingerprint: content.fingerprint,
                signature: content.signature,
                is_expanded,
                variant,
                wrapped_count,
                lines: Some(lines),
                last_touched: self.touch_counter,
            },
        );
    }

    /// Store a batch of measured wrapped line counts, one per entry.
    ///
    /// Used by the off-thread layout worker, which measures a whole session
    /// at once and hands back only the counts. The per-entry invalidation
    /// check is hoisted out of the loop: the batch is computed at a single
    /// width, so checking once and then storing all of them avoids clearing
    /// the cache on the first item and re-checking the width for every
    /// subsequent one.
    ///
    /// Each `MeasuredLineCount` carries the entry's content identity as the
    /// worker observed it, so storing a count never re-hashes the entry.
    pub fn insert_counts(&mut self, measured: &[MeasuredLineCount], content_width: u16) {
        let touch_counter = self.touch_counter;
        for count in measured {
            self.entries.insert(
                count.id.clone(),
                CachedEntryCount {
                    content_width,
                    fingerprint: count.content.fingerprint,
                    signature: count.content.signature,
                    is_expanded: count.is_expanded,
                    variant: count.variant,
                    wrapped_count: count.wrapped_count,
                    // Counts only: the rendered lines are what the worker's
                    // measurement deliberately threw away, and a later
                    // render miss repopulates them for the visible entries.
                    lines: None,
                    last_touched: touch_counter,
                },
            );
        }
    }

    /// Remove a specific entry from the cache.
    pub fn invalidate_entry(&mut self, id: &ChatEntryId) {
        self.entries.remove(id);
    }

    /// Clear the entire cache.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.touch_counter = 0;
        self.evictions = 0;
    }

    /// Number of entries currently cached.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
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
    use jinn_core_types::ToolResultStatus;
    use jinn_core_types::{ChatEntry, ChatEntryKind};

    /// Store a count the way the render pass does, via a probe.
    fn insert(
        entry: &ChatEntry,
        cache: &mut EntryLineCache,
        is_expanded: bool,
        variant: u64,
        width: u16,
        wrapped_count: u32,
    ) {
        let content = cache.probe(entry, is_expanded, variant, width).content;
        cache.insert(entry, content, is_expanded, variant, width, wrapped_count);
    }

    /// Store rendered lines the way the render pass does, via a probe.
    #[expect(
        clippy::rc_buffer,
        reason = "Arc keeps cloning a rendered entry's line buffer O(1) where a plain Vec would deep-copy every line on each cache hit"
    )]
    fn insert_with_lines(
        entry: &ChatEntry,
        cache: &mut EntryLineCache,
        is_expanded: bool,
        variant: u64,
        width: u16,
        wrapped_count: u32,
        lines: Arc<Vec<Line<'static>>>,
    ) {
        let content = cache.probe(entry, is_expanded, variant, width).content;
        cache.insert_with_lines(
            entry,
            content,
            is_expanded,
            variant,
            width,
            wrapped_count,
            lines,
        );
    }

    #[rstest::rstest]
    fn cache_hit_returns_count() {
        // Given an entry and a cache with its count.
        let mut cache = EntryLineCache::new();
        let entry = ChatEntry::assistant("hello");
        insert(&entry, &mut cache, false, 0, 80, 5);

        // When looking up the same entry.
        let result = cache.get(&entry, false, 0, 80);

        // Then the cached count is returned.
        assert_eq!(result.map(|h| h.wrapped_count), Some(5));
    }

    #[rstest::rstest]
    fn cache_miss_on_new_entry() {
        // Given an empty cache.
        let mut cache = EntryLineCache::new();
        let entry = ChatEntry::assistant("hello");

        // When looking up an uncached entry.
        let result = cache.get(&entry, false, 0, 80);

        // Then None is returned.
        assert!(result.is_none());
    }

    #[rstest::rstest]
    fn cache_miss_on_content_change() {
        // Given a cache with an entry's count.
        let mut cache = EntryLineCache::new();
        let mut entry = ChatEntry::assistant("hello");
        insert(&entry, &mut cache, false, 0, 80, 5);

        // When the entry's content changes.
        if let jinn_core_types::ChatEntryKind::Assistant(ref mut text) = entry.kind {
            text.push_str(" world");
        }

        // Then the cache misses (fingerprint mismatch).
        let result = cache.get(&entry, false, 0, 80);
        assert!(result.is_none());
    }

    #[rstest::rstest]
    fn cache_miss_on_expanded_change() {
        // Given a cache with an entry at is_expanded=false.
        let mut cache = EntryLineCache::new();
        let entry = ChatEntry::assistant("hello");
        insert(&entry, &mut cache, false, 0, 80, 5);

        // When looking up with is_expanded=true.
        let result = cache.get(&entry, true, 0, 80);

        // Then the cache misses.
        assert!(result.is_none());
    }

    #[rstest::rstest]
    fn cache_misses_on_content_width_change() {
        // Given a cache with an entry measured at width 80.
        let mut cache = EntryLineCache::new();
        let entry = ChatEntry::assistant("hello");
        insert(&entry, &mut cache, false, 0, 80, 5);

        // When looking up at width 100.
        let result = cache.get(&entry, false, 0, 100);

        // Then the count measured at 80 is not served at 100.
        assert!(result.is_none());
    }

    #[rstest::rstest]
    fn a_width_change_spares_entries_measured_at_the_new_width() {
        // Given a cache holding two sessions' entries, measured at 80 and 100.
        let mut cache = EntryLineCache::new();
        let at_eighty = ChatEntry::assistant("measured at eighty");
        let at_hundred = ChatEntry::assistant("measured at a hundred");
        insert(&at_eighty, &mut cache, false, 0, 80, 5);
        insert(&at_hundred, &mut cache, false, 0, 100, 9);

        // When the cache is used at width 100.
        let hit = cache.get(&at_hundred, false, 0, 100);

        // Then the entry measured at that width is still served.
        assert_eq!(hit.map(|h| h.wrapped_count), Some(9));
        // And the entry measured at the other width is still cached, not
        // discarded by the width it was measured at.
        assert!(
            !cache.is_empty(),
            "a width difference must not discard counts taken at another width"
        );
    }

    #[rstest::rstest]
    fn invalidate_entry_removes_specific_entry() {
        // Given a cache with two entries.
        let mut cache = EntryLineCache::new();
        let entry1 = ChatEntry::assistant("hello");
        let entry2 = ChatEntry::assistant("world");
        insert(&entry1, &mut cache, false, 0, 80, 3);
        insert(&entry2, &mut cache, false, 0, 80, 5);

        // When invalidating entry1.
        cache.invalidate_entry(&entry1.id);

        // Then entry1 is gone but entry2 remains.
        assert!(cache.get(&entry1, false, 0, 80).is_none());
        assert_eq!(
            cache.get(&entry2, false, 0, 80).map(|h| h.wrapped_count),
            Some(5)
        );
    }

    #[rstest::rstest]
    fn clear_removes_all_entries() {
        // Given a cache with entries.
        let mut cache = EntryLineCache::new();
        insert(&ChatEntry::assistant("hello"), &mut cache, false, 0, 80, 3);

        // When clearing.
        cache.clear();

        // Then the cache is empty.
        assert!(cache.is_empty());
    }

    #[rstest::rstest]
    fn fingerprint_stable_for_same_content() {
        // Given two entries with the same content.
        let entry1 = ChatEntry::assistant("hello");
        let entry2 = ChatEntry::assistant("hello");

        // Then their fingerprints match.
        assert_eq!(entry1.content_fingerprint(), entry2.content_fingerprint());
    }

    #[rstest::rstest]
    fn fingerprint_differs_for_different_content() {
        // Given two entries with different content.
        let entry1 = ChatEntry::assistant("hello");
        let entry2 = ChatEntry::assistant("world");

        // Then their fingerprints differ.
        assert_ne!(entry1.content_fingerprint(), entry2.content_fingerprint());
    }

    #[rstest::rstest]
    fn fingerprint_differs_for_different_kinds() {
        // Given entries of different kinds with same text.
        let assistant = ChatEntry::assistant("hello");
        let system = ChatEntry::system("hello");

        // Then their fingerprints differ.
        assert_ne!(
            assistant.content_fingerprint(),
            system.content_fingerprint()
        );
    }

    #[rstest::rstest]
    fn cache_hit_on_pending_tool_result_when_fingerprint_matches() {
        // Given a cache with a pending ToolResult entry.
        let mut cache = EntryLineCache::new();
        let entry = ChatEntry::tool_result("id", "bash", "", ToolResultStatus::Pending);
        insert(&entry, &mut cache, false, 0, 80, 5);

        // When looking up the pending entry with unchanged content.
        let result = cache.get(&entry, false, 0, 80);

        // Then the cached count is returned (pending entries are cacheable).
        assert_eq!(result.map(|h| h.wrapped_count), Some(5));
    }

    #[rstest::rstest]
    fn cache_miss_on_pending_tool_result_content_change() {
        // Given a cache with a pending ToolResult entry.
        let mut cache = EntryLineCache::new();
        let mut entry = ChatEntry::tool_result("id", "bash", "output", ToolResultStatus::Pending);
        insert(&entry, &mut cache, false, 0, 80, 5);

        // When the entry's content changes (simulating tool output growth).
        if let ChatEntryKind::ToolResult {
            ref mut content, ..
        } = entry.kind
        {
            content.push_str(" more");
        }

        let result = cache.get(&entry, false, 0, 80);

        // Then the cache misses (fingerprint mismatch).
        assert!(result.is_none());
    }

    #[rstest::rstest]
    fn cache_stores_and_returns_lines() {
        // Given an entry inserted with rendered lines.
        let mut cache = EntryLineCache::new();
        let entry = ChatEntry::assistant("hello");
        let lines = Arc::new(vec![Line::from("hello")]);
        insert_with_lines(&entry, &mut cache, false, 0, 80, 1, lines.clone());

        // When looking up the same entry.
        let result = cache.get(&entry, false, 0, 80);

        // Then the cached lines are returned.
        let hit = result.expect("should be a cache hit");
        assert_eq!(hit.wrapped_count, 1);
        let cached_lines = hit.lines.expect("should have cached lines");
        assert_eq!(*cached_lines, *lines);
    }

    #[rstest::rstest]
    fn cache_hit_without_lines_returns_none_lines() {
        // Given an entry inserted via insert() (no lines).
        let mut cache = EntryLineCache::new();
        let entry = ChatEntry::assistant("hello");
        insert(&entry, &mut cache, false, 0, 80, 5);

        // When looking up the same entry.
        let result = cache.get(&entry, false, 0, 80);

        // Then the count is returned but lines is None.
        let hit = result.expect("should be a cache hit");
        assert_eq!(hit.wrapped_count, 5);
        assert!(hit.lines.is_none());
    }

    #[rstest::rstest]
    fn cache_hit_when_variant_unchanged() {
        // Given a cache with an entry under variant 7.
        let mut cache = EntryLineCache::new();
        let entry = ChatEntry::assistant("hello");
        insert(&entry, &mut cache, false, 7, 80, 5);

        // When looking up with the same variant.
        let result = cache.get(&entry, false, 7, 80);

        // Then the cached count is returned.
        assert_eq!(result.map(|h| h.wrapped_count), Some(5));
    }

    #[rstest::rstest]
    fn cache_miss_when_variant_changes() {
        // Given a cache with an entry under variant 7.
        let mut cache = EntryLineCache::new();
        let entry = ChatEntry::assistant("hello");
        insert(&entry, &mut cache, false, 7, 80, 5);

        // When looking up with a different variant.
        let result = cache.get(&entry, false, 8, 80);

        // Then the cache misses.
        assert!(result.is_none());
    }
    #[rstest::rstest]
    fn repeat_lookup_of_unchanged_entry_computes_no_fingerprint() {
        // Given a cache warmed with a large entry.
        let mut cache = EntryLineCache::new();
        let entry =
            ChatEntry::tool_result("id", "bash", "x".repeat(50_000), ToolResultStatus::Success);
        insert(&entry, &mut cache, false, 0, 80, 40);

        // When looking it up many times, as a frame does per visible entry.
        for _ in 0..1_000 {
            assert!(cache.get(&entry, false, 0, 80).is_some());
        }

        // Then no full fingerprint was computed beyond the initial one.
        assert_eq!(
            cache.fingerprint_computations(),
            1,
            "steady-state lookups should reuse the cached fingerprint"
        );
    }

    #[rstest::rstest]
    fn changed_entry_length_computes_one_fingerprint_per_lookup() {
        // Given a cached entry whose content later changes length.
        let mut cache = EntryLineCache::new();
        let mut entry = ChatEntry::tool_result("id", "bash", "line one", ToolResultStatus::Success);
        insert(&entry, &mut cache, false, 0, 80, 4);

        // When the content grows, as streaming tool output does.
        if let ChatEntryKind::ToolResult {
            ref mut content, ..
        } = entry.kind
        {
            content.push_str(" plus more output");
        }

        // Then the lookup misses and did hash once to discover the change.
        assert!(cache.get(&entry, false, 0, 80).is_none());
        assert_eq!(cache.fingerprint_computations(), 2);
    }

    #[rstest::rstest]
    fn storing_a_missed_entry_does_not_rehash_it() {
        // Given a cache that missed on an entry.
        let mut cache = EntryLineCache::new();
        let entry =
            ChatEntry::tool_result("id", "bash", "y".repeat(50_000), ToolResultStatus::Success);
        let probe = cache.probe(&entry, false, 0, 80);

        // When storing the count using the identity from that same probe.
        cache.insert(&entry, probe.content, false, 0, 80, 10);

        // Then the entry was hashed exactly once, not once per phase.
        assert_eq!(
            cache.fingerprint_computations(),
            1,
            "the insert should reuse the probe's fingerprint"
        );
    }

    #[rstest::rstest]
    fn cached_lines_survive_a_fingerprint_skipped_hit() {
        // Given an entry cached with rendered lines.
        let mut cache = EntryLineCache::new();
        let entry = ChatEntry::assistant("hello world");
        let lines = Arc::new(vec![Line::from("hello world")]);
        insert_with_lines(&entry, &mut cache, false, 0, 80, 1, lines.clone());

        // When it is looked up again unchanged.
        let hit = cache.get(&entry, false, 0, 80);

        // Then the same lines come back, so skipping the hash changed nothing.
        let hit = hit.expect("should be a cache hit");
        assert_eq!(hit.wrapped_count, 1);
        assert_eq!(*hit.lines.expect("should have lines"), *lines);
    }
    /// Fill the cache with `n` entries holding rendered lines, touching each
    /// in insertion order so recency matches insertion order.
    fn fill_with_lines(cache: &mut EntryLineCache, n: usize) -> Vec<ChatEntry> {
        (0..n)
            .map(|i| {
                let entry = ChatEntry::assistant(format!("entry {i}"));
                insert_with_lines(
                    &entry,
                    cache,
                    false,
                    0,
                    80,
                    1,
                    Arc::new(vec![Line::from(format!("entry {i}"))]),
                );
                cache.touch(&entry.id);
                entry
            })
            .collect()
    }

    #[rstest::rstest]
    fn eviction_drops_the_least_recently_touched_lines() {
        // Given more entries with lines than the cache retains.
        let mut cache = EntryLineCache::new();
        let entries = fill_with_lines(&mut cache, MAX_CACHED_RENDERED_ENTRIES + 5);

        // When eviction runs.
        cache.evict_if_needed();

        // Then the oldest entries lose their lines but keep their counts.
        for entry in &entries[..5] {
            let hit = cache
                .get(entry, false, 0, 80)
                .expect("count should survive");
            assert!(
                hit.lines.is_none(),
                "the least recently used entry should have lost its lines"
            );
        }
    }

    #[rstest::rstest]
    fn eviction_keeps_the_most_recently_touched_lines() {
        // Given a cache one entry over its limit.
        let mut cache = EntryLineCache::new();
        let entries = fill_with_lines(&mut cache, MAX_CACHED_RENDERED_ENTRIES + 1);
        let newest = entries.last().expect("entries were just created");

        // When eviction runs.
        cache.evict_if_needed();

        // Then the newest entry still has its lines.
        let hit = cache
            .get(newest, false, 0, 80)
            .expect("count should survive");
        assert!(
            hit.lines.is_some(),
            "the most recently used entry should keep its lines"
        );
    }

    #[rstest::rstest]
    fn eviction_never_drops_a_wrapped_count() {
        // Given more entries with lines than the cache retains.
        let mut cache = EntryLineCache::new();
        let entries = fill_with_lines(&mut cache, MAX_CACHED_RENDERED_ENTRIES + 10);

        // When eviction runs.
        cache.evict_if_needed();

        // Then every entry still reports its wrapped count, evicted or not.
        for entry in &entries {
            assert_eq!(
                cache
                    .get(entry, false, 0, 80)
                    .expect("every entry keeps a count")
                    .wrapped_count,
                1,
                "eviction must never cost a re-layout"
            );
        }
        assert!(cache.evictions() > 0, "eviction should have run");
    }

    #[rstest::rstest]
    fn touching_an_entry_protects_it_from_eviction() {
        // Given a cache at its limit, then re-using the oldest entry.
        let mut cache = EntryLineCache::new();
        let entries = fill_with_lines(&mut cache, MAX_CACHED_RENDERED_ENTRIES);
        let oldest = entries.first().expect("entries were just created");
        cache.touch(&oldest.id);

        // When one more entry pushes the cache over the limit.
        let newcomer = ChatEntry::assistant("newcomer");
        insert_with_lines(
            &newcomer,
            &mut cache,
            false,
            0,
            80,
            1,
            Arc::new(vec![Line::from("newcomer")]),
        );
        cache.touch(&newcomer.id);
        cache.evict_if_needed();

        // Then the re-used entry survived, because a scrolled-to entry is
        // what the user is actually looking at.
        let hit = cache
            .get(oldest, false, 0, 80)
            .expect("count should survive");
        assert!(
            hit.lines.is_some(),
            "a recently scrolled-to entry should not be evicted"
        );
    }

    #[rstest::rstest]
    fn a_width_change_does_not_reset_the_eviction_tally() {
        // Given a cache holding entries and having evicted at least once.
        let mut cache = EntryLineCache::new();
        fill_with_lines(&mut cache, MAX_CACHED_RENDERED_ENTRIES + 1);
        cache.evict_if_needed();
        let evictions = cache.evictions();
        assert!(evictions > 0, "the fixture must have evicted at least once");

        // When the content width changes and an entry is looked up.
        let entry = ChatEntry::assistant("hello");
        let result = cache.get(&entry, false, 0, 100);

        // Then that entry misses, but the cache keeps the counts it holds.
        assert!(result.is_none());
        // And the eviction history is a running tally, not a per-width one.
        assert_eq!(
            cache.evictions(),
            evictions,
            "a width difference must not erase the eviction tally"
        );
    }

    #[rstest::rstest]
    fn an_evicted_entry_repopulates_when_rendered_again() {
        // Given a cache whose entry has had its lines evicted.
        let mut cache = EntryLineCache::new();
        let entry = ChatEntry::assistant("repopulated content");
        insert_with_lines(
            &entry,
            &mut cache,
            false,
            0,
            80,
            1,
            Arc::new(vec![Line::from("repopulated content")]),
        );
        if let Some(cached) = cache.entries.get_mut(&entry.id) {
            cached.lines = None;
        }

        // When the entry is looked up.
        let hit = cache
            .get(&entry, false, 0, 80)
            .expect("count survives eviction");

        // Then the count is still served and the caller can re-render.
        assert_eq!(hit.wrapped_count, 1);
        assert!(hit.lines.is_none(), "lines were evicted, so it re-renders");
    }
}
