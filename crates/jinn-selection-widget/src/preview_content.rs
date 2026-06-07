//! Preview content trait - provides renderable lines for a preview pane.
//!
//! Items that want to show a preview in the picker implement this trait.
//! The preview widget calls [`PreviewContent::preview_lines`] (or the caching
//! variant [`PreviewContent::preview_lines_cached`]) to get styled lines for
//! display.

use ratatui::text::Line;

/// Abstract cache for rendered preview lines.
///
/// Lets a [`PreviewContent`] implementor skip re-rendering when the same
/// `(key, width)` has already been rendered. Concrete implementations live in
/// consumer crates; this crate only defines the contract so the preview widget
/// can be wired to a cache without depending on any domain type.
pub trait PreviewCache {
    /// Looks up previously rendered lines for the given key and width.
    ///
    /// Returns an owned clone so callers may mutate the result without aliasing
    /// the stored value. Returns `None` on a miss.
    fn get(&self, key: &str, width: usize) -> Option<Vec<Line<'static>>>;

    /// Stores rendered lines for the given key and width.
    ///
    /// Implementations use interior mutability (e.g. `RefCell`/`Mutex`) so the
    /// cache can be shared behind an immutable `&self` reference. This keeps the
    /// `PreviewCache` covariant over its lifetime, allowing reborrowing from a
    /// transient borrow without variance or lifetime conflicts.
    fn insert(&self, key: String, width: usize, lines: Vec<Line<'static>>);
}

/// Trait for picker items that can provide preview content.
///
/// Implementors return styled lines for display in the preview pane.
/// The `width` parameter allows word-wrapping to the available pane width.
///
/// # Caching
///
/// Expensive previews (e.g. markdown rendering with syntax highlighting) can opt
/// into caching by overriding [`cache_key`](Self::cache_key) to return `Some`.
/// The preview widget then calls [`preview_lines_cached`](Self::preview_lines_cached)
/// instead of [`preview_lines`](Self::preview_lines) when a [`PreviewCache`] is
/// available, avoiding redundant re-renders across frames.
pub trait PreviewContent {
    /// Returns the preview lines for display in the preview pane.
    ///
    /// `width` is the number of columns available for rendering.
    /// Implementors should wrap text to fit within this width.
    fn preview_lines(&self, width: usize) -> Vec<Line<'static>>;

    /// Identity key used to cache rendered preview lines.
    ///
    /// Returns `None` by default, meaning "never cache this item." Implementors
    /// that want caching override this to return a stable, unique key (e.g. an
    /// item name or id). The key is combined with `width` to form the cache
    /// entry key.
    fn cache_key(&self) -> Option<String> {
        None
    }

    /// Get-or-render-and-insert, delegating to a [`PreviewCache`] when one is
    /// supplied.
    ///
    /// Behavior:
    /// - If [`cache_key`](Self::cache_key) returns `None`, always renders fresh
    ///   via [`preview_lines`](Self::preview_lines) (item opted out of caching).
    /// - If `cache` is `None`, always renders fresh (caller supplied no cache).
    /// - Otherwise, a cache hit returns the stored lines without re-rendering;
    ///   a miss renders, stores, and returns the lines.
    fn preview_lines_cached(
        &self,
        width: usize,
        cache: Option<&dyn PreviewCache>,
    ) -> Vec<Line<'static>> {
        let Some(key) = self.cache_key() else {
            return self.preview_lines(width);
        };
        match cache {
            None => self.preview_lines(width),
            Some(c) => match c.get(&key, width) {
                Some(cached) => cached,
                None => {
                    let lines = self.preview_lines(width);
                    c.insert(key, width, lines.clone());
                    lines
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use std::cell::Cell;

    /// A test item that counts how many times `preview_lines` runs.
    struct CountingItem {
        key: Option<String>,
        render_count: Cell<usize>,
    }

    impl CountingItem {
        fn new(key: Option<String>) -> Self {
            Self {
                key,
                render_count: Cell::new(0),
            }
        }

        fn render_count(&self) -> usize {
            self.render_count.get()
        }
    }

    impl PreviewContent for CountingItem {
        fn preview_lines(&self, _width: usize) -> Vec<Line<'static>> {
            self.render_count.set(self.render_count.get() + 1);
            vec![Line::from("rendered")]
        }
        fn cache_key(&self) -> Option<String> {
            self.key.clone()
        }
    }

    /// Simple in-test [`PreviewCache`] backed by a `HashMap`.
    #[derive(Default)]
    struct TestCache {
        entries: std::cell::RefCell<std::collections::HashMap<(String, usize), Vec<Line<'static>>>>,
    }

    impl PreviewCache for TestCache {
        fn get(&self, key: &str, width: usize) -> Option<Vec<Line<'static>>> {
            self.entries.borrow().get(&(key.to_owned(), width)).cloned()
        }
        fn insert(&self, key: String, width: usize, lines: Vec<Line<'static>>) {
            self.entries.borrow_mut().insert((key, width), lines);
        }
    }

    #[rstest::rstest]
    fn cache_miss_then_hit_avoids_second_render() {
        // Given an item with a cache key and a fresh cache.
        let item = CountingItem::new(Some("alpha".to_owned()));
        let cache = TestCache::default();

        // When calling cached twice at the same width.
        let _first = item.preview_lines_cached(80, Some(&cache));
        let _second = item.preview_lines_cached(80, Some(&cache));

        // Then preview_lines ran only once (first was a miss, second a hit).
        assert_eq!(item.render_count(), 1);
    }

    #[rstest::rstest]
    fn different_width_is_a_cache_miss() {
        // Given an item with a cache key.
        let item = CountingItem::new(Some("alpha".to_owned()));
        let cache = TestCache::default();

        // When calling cached at two different widths.
        let _w1 = item.preview_lines_cached(80, Some(&cache));
        let _w2 = item.preview_lines_cached(100, Some(&cache));

        // Then preview_lines ran twice (width is part of the key).
        assert_eq!(item.render_count(), 2);
    }

    #[rstest::rstest]
    fn none_cache_key_always_renders() {
        // Given an item whose cache_key returns None (default).
        let item = CountingItem::new(None);
        let cache = TestCache::default();

        // When calling cached twice with a cache supplied.
        let _first = item.preview_lines_cached(80, Some(&cache));
        let _second = item.preview_lines_cached(80, Some(&cache));

        // Then preview_lines runs both times (item opted out of caching).
        assert_eq!(item.render_count(), 2);
        assert!(
            cache.entries.borrow().is_empty(),
            "no entry should be inserted when cache_key is None"
        );
    }

    #[rstest::rstest]
    fn no_cache_supplied_always_renders() {
        // Given an item with a cache key.
        let item = CountingItem::new(Some("alpha".to_owned()));

        // When calling cached twice without a cache.
        let _first = item.preview_lines_cached(80, None);
        let _second = item.preview_lines_cached(80, None);

        // Then preview_lines runs both times.
        assert_eq!(item.render_count(), 2);
    }

    #[rstest::rstest]
    fn stored_lines_are_returned_on_hit() {
        // Given an item with a cache key.
        let item = CountingItem::new(Some("alpha".to_owned()));
        let cache = TestCache::default();

        // When calling cached twice.
        let first = item.preview_lines_cached(80, Some(&cache));
        let second = item.preview_lines_cached(80, Some(&cache));

        // Then both calls return lines (content equivalence via length).
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
    }
}
