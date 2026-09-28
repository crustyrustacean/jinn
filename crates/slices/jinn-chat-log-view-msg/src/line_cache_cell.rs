//! The chat log's per-entry line cache, as a slice cell.
//!
//! The cache used to sit on `AppState` as a bare `RwLock`, which made it
//! the one piece of theme-sensitive rendering state no slice owned. It is
//! now a cell in this crate — the slice that writes it, and the one place
//! a theme change has to reach — and a `RwLock` inside the payload keeps
//! the render pass's borrow scoped to a single frame.

use jinn_slices::SlotKey;

use crate::line_count_cache::EntryLineCache;

/// The slot key the chat log's line cache lives under.
#[must_use]
pub fn entry_line_cache_slot() -> SlotKey {
    SlotKey::builtin("chat-log-view", "line-cache")
}

/// The chat-log-view slice's line-cache cell payload.
///
/// Wraps [`EntryLineCache`] in a lock so the cell is the one handle the
/// render pass, the layout worker, and the theme-change path all share.
/// Entries are keyed by entry id, so the cache spans every open session.
#[derive(Debug, Default)]
pub struct ChatLogLineCache {
    entries: parking_lot::RwLock<EntryLineCache>,
}

impl ChatLogLineCache {
    /// Creates an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs `f` against the cache under its write lock.
    ///
    /// The single mutation path. The closure runs to completion before the
    /// lock is released, so a reader never observes a half-applied update.
    pub fn update<F, R>(&self, f: F) -> R
    where
        R: Sized,
        F: FnOnce(&mut EntryLineCache) -> R,
    {
        f(&mut self.entries.write())
    }

    /// Runs `f` against the cache under its read lock.
    #[must_use]
    pub fn read<F, R>(&self, f: F) -> R
    where
        R: Sized,
        F: FnOnce(&EntryLineCache) -> R,
    {
        f(&self.entries.read())
    }

    /// Drops every cached entry.
    ///
    /// The theme-change path: cached lines embed the old palette's colours,
    /// so a theme switch must not leave them behind.
    pub fn clear(&self) {
        self.update(EntryLineCache::clear);
    }
}
