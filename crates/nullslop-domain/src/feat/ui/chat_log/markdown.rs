//! Markdown rendering adapter for chat log entries.
//!
//! Bridges the `ratatui-markdown` crate's [`RichTextTheme`] trait to nullslop's
//! [`Theme`] struct, and provides a [`render_markdown`] helper that produces
//! `Vec<Line<'static>>` ready for the chat log renderer.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use ratatui::text::Line;
use ratatui_markdown::highlight::{HighlightHooks, TreeSitterHighlighter};
use ratatui_markdown::markdown::{MarkdownBlock, MarkdownRenderer, RenderHooks};
use ratatui_markdown::theme::{Generation, RichTextTheme};

use crate::feat::theme::Theme;

/// Render markdown text into styled lines for display in the chat log.
///
/// Creates a [`MarkdownRenderer`] with syntax highlighting hooks, parses the
/// markdown, and renders it using the nullslop theme. The `width` parameter
/// controls word-wrapping.
pub fn render_markdown(text: &str, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let text = text.trim();
    let renderer = MarkdownRenderer::new(width as usize)
        .with_render_hooks(highlight_hooks(width as usize, theme));
    let blocks = renderer.parse(text);
    renderer.render(&blocks, theme)
}

/// Build the render hooks for syntax-highlighted code blocks.
fn highlight_hooks(max_width: usize, theme: &Theme) -> Box<dyn RenderHooks> {
    let highlighter = Arc::new(TreeSitterHighlighter::new());
    let hooks = HighlightHooks::new(highlighter, max_width).with_border_color(theme.muted_text);
    Box::new(hooks)
}

// ---------------------------------------------------------------------------
// Markdown AST cache
// ---------------------------------------------------------------------------

/// Cache for parsed markdown ASTs, keyed by text content hash and theme generation.
///
/// Avoids re-parsing markdown for entries whose text hasn't changed across
/// render frames. The cache lives on [`ChatLogElement`](super::ChatLogElement)
/// alongside [`EntryLineCache`](super::EntryLineCache).
///
/// # Invalidation
///
/// - **Theme generation change:** clears all entries (theme colors affect rendering,
///   though not the AST itself — keying on generation ensures correctness when
///   theme hot-reload is implemented).
/// - **Content change:** automatic miss (hash mismatch).
/// - **Streaming entries:** automatic miss (text changes every frame).
#[derive(Debug, Clone, Default)]
pub struct MarkdownAstCache {
    entries: HashMap<u64, CachedAst>,
    generation: Option<Generation>,
}

#[derive(Debug, Clone)]
struct CachedAst {
    blocks: Arc<Vec<MarkdownBlock>>,
}

impl MarkdownAstCache {
    /// Create a new empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a cached AST for the given text.
    ///
    /// Returns `None` if no cache entry exists, the text hash doesn't match,
    /// or the theme generation has changed (which also clears the entire cache).
    pub fn get(&mut self, text: &str, generation: Generation) -> Option<Arc<Vec<MarkdownBlock>>> {
        if self.generation != Some(generation) {
            self.entries.clear();
            self.generation = Some(generation);
            return None;
        }
        let key = Self::hash_text(text);
        self.entries.get(&key).map(|cached| cached.blocks.clone())
    }

    /// Store a parsed AST for the given text.
    ///
    /// If the theme generation has changed, clears the cache before inserting.
    pub fn insert(
        &mut self,
        text: &str,
        generation: Generation,
        blocks: Arc<Vec<MarkdownBlock>>,
    ) {
        if self.generation != Some(generation) {
            self.entries.clear();
            self.generation = Some(generation);
        }
        let key = Self::hash_text(text);
        self.entries.insert(key, CachedAst { blocks });
    }

    /// Hash the text for use as a cache key.
    fn hash_text(text: &str) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        hasher.finish()
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

impl RichTextTheme for Theme {
    fn generation(&self) -> Generation {
        Generation(1)
    }

    fn get_text_color(&self) -> ratatui::style::Color {
        self.primary_text
    }

    fn get_muted_text_color(&self) -> ratatui::style::Color {
        self.muted_text
    }

    fn get_primary_color(&self) -> ratatui::style::Color {
        self.focus_accent
    }

    fn get_popup_selected_background(&self) -> ratatui::style::Color {
        self.focus_accent
    }

    fn get_popup_selected_text_color(&self) -> ratatui::style::Color {
        self.primary_text
    }

    fn get_border_color(&self) -> ratatui::style::Color {
        self.border_unfocused
    }

    fn get_focused_border_color(&self) -> ratatui::style::Color {
        self.focus_accent
    }

    fn get_secondary_color(&self) -> ratatui::style::Color {
        self.success
    }

    fn get_info_color(&self) -> ratatui::style::Color {
        self.streaming
    }

    fn get_background_color(&self) -> ratatui::style::Color {
        self.user_block_bg
    }

    fn get_json_key_color(&self) -> ratatui::style::Color {
        self.focus_accent
    }

    fn get_json_string_color(&self) -> ratatui::style::Color {
        self.success
    }

    fn get_json_number_color(&self) -> ratatui::style::Color {
        self.warning
    }

    fn get_json_bool_color(&self) -> ratatui::style::Color {
        self.streaming
    }

    fn get_json_null_color(&self) -> ratatui::style::Color {
        self.muted_text
    }

    fn get_accent_yellow(&self) -> ratatui::style::Color {
        self.warning
    }
}

#[cfg(test)]
mod cache_tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;

    fn parse_blocks(text: &str) -> Arc<Vec<MarkdownBlock>> {
        let renderer = MarkdownRenderer::new(80);
        Arc::new(renderer.parse(text))
    }

    #[rstest::rstest]
    fn cache_hit_returns_same_blocks_as_fresh_parse() {
        // Given a cache with parsed AST for "Hello **world**".
        let mut cache = MarkdownAstCache::new();
        let blocks = parse_blocks("Hello **world**");
        cache.insert("Hello **world**", Generation(1), blocks);

        // When calling get() with same text and same generation.
        let result = cache.get("Hello **world**", Generation(1));

        // Then returns Some with identical blocks.
        let cached = result.expect("should have a cache hit");
        let fresh = parse_blocks("Hello **world**");
        assert_eq!(*cached, *fresh);
    }

    #[rstest::rstest]
    fn cache_miss_on_different_text() {
        // Given a cache with parsed AST for "Hello".
        let mut cache = MarkdownAstCache::new();
        cache.insert("Hello", Generation(1), parse_blocks("Hello"));

        // When calling get() with "World" and same generation.
        let result = cache.get("World", Generation(1));

        // Then returns None.
        assert!(result.is_none());
    }

    #[rstest::rstest]
    fn cache_miss_on_generation_change_clears_cache() {
        // Given a cache populated at Generation(1).
        let mut cache = MarkdownAstCache::new();
        cache.insert("Hello", Generation(1), parse_blocks("Hello"));
        assert_eq!(cache.len(), 1);

        // When calling get() with Generation(2).
        let result = cache.get("Hello", Generation(2));

        // Then returns None AND cache is cleared.
        assert!(result.is_none());
        assert!(cache.is_empty());
    }

    #[rstest::rstest]
    fn cache_insert_stores_blocks() {
        // Given an empty cache.
        let mut cache = MarkdownAstCache::new();

        // When inserting parsed blocks for "Hello".
        cache.insert("Hello", Generation(1), parse_blocks("Hello"));

        // Then subsequent get() with same text returns the blocks.
        let result = cache.get("Hello", Generation(1));
        assert!(result.is_some());
    }

    #[rstest::rstest]
    fn cache_handles_two_different_texts() {
        // Given a cache with two entries.
        let mut cache = MarkdownAstCache::new();
        cache.insert("Hello", Generation(1), parse_blocks("Hello"));
        cache.insert("World", Generation(1), parse_blocks("World"));

        // Then both are retrievable.
        assert!(cache.get("Hello", Generation(1)).is_some());
        assert!(cache.get("World", Generation(1)).is_some());
        assert_eq!(cache.len(), 2);
    }
}
