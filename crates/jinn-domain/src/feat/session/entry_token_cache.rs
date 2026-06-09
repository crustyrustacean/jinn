//! Cache of tiktoken-based token counts per chat entry.
//!
//! [`EntryTokenCache`] maps chat entry IDs to their estimated token counts.
//! Populated by the token count actor, read by the minimap render pipeline.
//! Not invalidated on theme change - token counts are theme-independent.

use std::collections::HashMap;

use crate::protocol::ChatEntryId;

/// Cache of tiktoken-based token counts per chat entry.
///
/// Stored in [`FrontendCaches`](crate::common::app_state::FrontendCaches) as
/// `RwLock<EntryTokenCache>`. The token count actor writes counts; the minimap
/// render pipeline reads them. Counts are not persisted - re-computed on session load.
#[derive(Debug, Clone, Default)]
pub struct EntryTokenCache {
    entries: HashMap<ChatEntryId, u32>,
}

impl EntryTokenCache {
    /// Look up the cached token count for an entry.
    pub fn get(&self, id: &ChatEntryId) -> Option<u32> {
        self.entries.get(id).copied()
    }

    /// Store a token count for an entry.
    pub fn insert(&mut self, id: ChatEntryId, count: u32) {
        self.entries.insert(id, count);
    }

    /// Whether a count has been cached for this entry.
    pub fn contains(&self, id: &ChatEntryId) -> bool {
        self.entries.contains_key(id)
    }

    /// Remove all cached counts.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Store token counts for multiple entries at once.
    ///
    /// Used by the token count actor for batch insertion after
    /// session load or history append.
    pub fn bulk_insert(&mut self, entries: impl IntoIterator<Item = (ChatEntryId, u32)>) {
        for (id, count) in entries {
            self.entries.insert(id, count);
        }
    }
}

#[cfg(test)]
mod tests {
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, clippy::string_slice, clippy::uninlined_format_args, reason = "test code")]
    use super::*;
    use crate::protocol::ChatEntry;

    #[rstest::rstest]
    fn get_returns_inserted_count() {
        // Given a cache with one entry.
        let mut cache = EntryTokenCache::default();
        let entry = ChatEntry::user("hello");
        cache.insert(entry.id.clone(), 42);

        // When looking up the entry.
        let result = cache.get(&entry.id);

        // Then the count is returned.
        assert_eq!(result, Some(42));
    }

    #[rstest::rstest]
    fn get_returns_none_for_missing() {
        // Given an empty cache.
        let cache = EntryTokenCache::default();
        let entry = ChatEntry::user("hello");

        // When looking up the entry.
        let result = cache.get(&entry.id);

        // Then None is returned.
        assert_eq!(result, None);
    }

    #[rstest::rstest]
    fn contains_returns_true_for_inserted() {
        // Given a cache with one entry.
        let mut cache = EntryTokenCache::default();
        let entry = ChatEntry::user("hello");
        cache.insert(entry.id.clone(), 42);

        // When checking membership.
        assert!(cache.contains(&entry.id));
        // And a different ID is not contained.
        let other = ChatEntry::user("other");
        assert!(!cache.contains(&other.id));
    }

    #[rstest::rstest]
    fn bulk_insert_adds_multiple_entries() {
        // Given an empty cache.
        let mut cache = EntryTokenCache::default();
        let entry1 = ChatEntry::user("hello");
        let entry2 = ChatEntry::assistant("world");

        // When bulk inserting.
        cache.bulk_insert([(entry1.id.clone(), 10), (entry2.id.clone(), 20)]);

        // Then both entries are present.
        assert_eq!(cache.get(&entry1.id), Some(10));
        assert_eq!(cache.get(&entry2.id), Some(20));
    }

    #[rstest::rstest]
    fn clear_removes_all_entries() {
        // Given a cache with entries.
        let mut cache = EntryTokenCache::default();
        let entry = ChatEntry::user("hello");
        cache.insert(entry.id.clone(), 42);

        // When clearing.
        cache.clear();

        // Then the cache is empty.
        assert!(!cache.contains(&entry.id));
    }

    #[rstest::rstest]
    fn insert_overwrites_existing() {
        // Given a cache with one entry.
        let mut cache = EntryTokenCache::default();
        let entry = ChatEntry::user("hello");
        cache.insert(entry.id.clone(), 42);

        // When re-inserting with a different count.
        cache.insert(entry.id.clone(), 99);

        // Then the new count is returned.
        assert_eq!(cache.get(&entry.id), Some(99));
    }
}
