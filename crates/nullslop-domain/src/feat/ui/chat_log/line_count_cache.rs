//! Line count cache for virtualized chat log rendering.
//!
//! Caches the wrapped line count per entry so the renderer can cheaply
//! determine which entries are visible without calling `entry_to_lines()`
//! for the entire history. The cache is invalidated on content changes
//! (streaming tokens), expand/collapse toggles, and content width changes
//! (terminal resize).

use std::collections::HashMap;

use crate::feat::session::tool_result_status::ToolResultStatus;
use crate::protocol::{ChatEntry, ChatEntryId, ChatEntryKind};

/// Cached wrapped line count for a single entry.
#[derive(Debug, Clone)]
pub struct CachedEntryCount {
    /// Fingerprint of the entry's content when this count was computed.
    pub fingerprint: u64,
    /// Whether the entry was expanded when this count was computed.
    pub is_expanded: bool,
    /// The wrapped line count for this entry.
    pub wrapped_count: u16,
}

/// Cache mapping entry IDs to their cached wrapped line counts.
///
/// Owned by `ChatLogElement` — populated during the render pass, used
/// to determine which entries overlap the viewport without re-rendering
/// the entire history.
///
/// # Invalidation
///
/// - **Content width change:** clears all entries via `set_content_width()`.
/// - **Streaming (content change):** detected by fingerprint mismatch → automatic miss.
/// - **Expand/collapse:** detected by `is_expanded` mismatch → automatic miss.
/// - **New entry:** no cache entry exists → automatic miss.
#[derive(Debug, Clone, Default)]
pub struct EntryLineCache {
    /// The content width used when cache entries were computed.
    /// If the current width differs, the entire cache is invalid.
    content_width: Option<u16>,
    /// Per-entry cached counts.
    entries: HashMap<ChatEntryId, CachedEntryCount>,
}

impl EntryLineCache {
    /// Create a new empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up the cached wrapped line count for an entry.
    ///
    /// Returns `None` if:
    /// - No cache entry exists for this ID (new entry).
    /// - The entry's fingerprint has changed (content changed during streaming).
    /// - The entry's expanded state has changed (expand/collapse toggle).
    /// - The content width has changed (terminal resize).
    pub fn get(&mut self, entry: &ChatEntry, is_expanded: bool, content_width: u16) -> Option<u16> {
        // Never cache pending tool result entries — their content changes every tick.
        if let ChatEntryKind::ToolResult { status, .. } = &entry.kind
            && *status == ToolResultStatus::Pending
        {
            return None;
        }

        // If content width changed, clear everything.
        if self.content_width != Some(content_width) {
            self.entries.clear();
            self.content_width = Some(content_width);
            return None;
        }

        let cached = self.entries.get(&entry.id)?;
        if cached.fingerprint == entry.content_fingerprint() && cached.is_expanded == is_expanded {
            Some(cached.wrapped_count)
        } else {
            None
        }
    }

    /// Store a wrapped line count for an entry.
    pub fn insert(
        &mut self,
        entry: &ChatEntry,
        is_expanded: bool,
        content_width: u16,
        wrapped_count: u16,
    ) {
        // Sync content width.
        if self.content_width != Some(content_width) {
            self.entries.clear();
            self.content_width = Some(content_width);
        }
        self.entries.insert(
            entry.id.clone(),
            CachedEntryCount {
                fingerprint: entry.content_fingerprint(),
                is_expanded,
                wrapped_count,
            },
        );
    }

    /// Remove a specific entry from the cache.
    #[allow(dead_code)]
    pub fn invalidate_entry(&mut self, id: &ChatEntryId) {
        self.entries.remove(id);
    }

    /// Clear the entire cache.
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.entries.clear();
        self.content_width = None;
    }

    /// Number of entries currently cached.
    #[must_use]
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ChatEntry;

    #[rstest::rstest]
    fn cache_hit_returns_count() {
        // Given an entry and a cache with its count.
        let mut cache = EntryLineCache::new();
        let entry = ChatEntry::assistant("hello");
        cache.insert(&entry, false, 80, 5);

        // When looking up the same entry.
        let result = cache.get(&entry, false, 80);

        // Then the cached count is returned.
        assert_eq!(result, Some(5));
    }

    #[rstest::rstest]
    fn cache_miss_on_new_entry() {
        // Given an empty cache.
        let mut cache = EntryLineCache::new();
        let entry = ChatEntry::assistant("hello");

        // When looking up an uncached entry.
        let result = cache.get(&entry, false, 80);

        // Then None is returned.
        assert!(result.is_none());
    }

    #[rstest::rstest]
    fn cache_miss_on_content_change() {
        // Given a cache with an entry's count.
        let mut cache = EntryLineCache::new();
        let mut entry = ChatEntry::assistant("hello");
        cache.insert(&entry, false, 80, 5);

        // When the entry's content changes.
        if let crate::protocol::ChatEntryKind::Assistant(ref mut text) = entry.kind {
            text.push_str(" world");
        }

        // Then the cache misses (fingerprint mismatch).
        let result = cache.get(&entry, false, 80);
        assert!(result.is_none());
    }

    #[rstest::rstest]
    fn cache_miss_on_expanded_change() {
        // Given a cache with an entry at is_expanded=false.
        let mut cache = EntryLineCache::new();
        let entry = ChatEntry::assistant("hello");
        cache.insert(&entry, false, 80, 5);

        // When looking up with is_expanded=true.
        let result = cache.get(&entry, true, 80);

        // Then the cache misses.
        assert!(result.is_none());
    }

    #[rstest::rstest]
    fn cache_cleared_on_content_width_change() {
        // Given a cache with entries at width 80.
        let mut cache = EntryLineCache::new();
        let entry = ChatEntry::assistant("hello");
        cache.insert(&entry, false, 80, 5);

        // When looking up at width 100.
        let result = cache.get(&entry, false, 100);

        // Then the cache misses (and is cleared).
        assert!(result.is_none());
        assert!(cache.is_empty());
    }

    #[rstest::rstest]
    fn invalidate_entry_removes_specific_entry() {
        // Given a cache with two entries.
        let mut cache = EntryLineCache::new();
        let entry1 = ChatEntry::assistant("hello");
        let entry2 = ChatEntry::assistant("world");
        cache.insert(&entry1, false, 80, 3);
        cache.insert(&entry2, false, 80, 5);

        // When invalidating entry1.
        cache.invalidate_entry(&entry1.id);

        // Then entry1 is gone but entry2 remains.
        assert!(cache.get(&entry1, false, 80).is_none());
        assert_eq!(cache.get(&entry2, false, 80), Some(5));
    }

    #[rstest::rstest]
    fn clear_removes_all_entries() {
        // Given a cache with entries.
        let mut cache = EntryLineCache::new();
        cache.insert(&ChatEntry::assistant("hello"), false, 80, 3);

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
    fn cache_miss_on_pending_tool_result() {
        // Given a cache with a pending ToolResult entry.
        let mut cache = EntryLineCache::new();
        let entry = ChatEntry::tool_result("id", "bash", "", ToolResultStatus::Pending);
        cache.insert(&entry, false, 80, 5);

        // When looking up the pending entry.
        let result = cache.get(&entry, false, 80);

        // Then the cache returns None (pending entries are never cached).
        assert!(result.is_none());
    }
}
