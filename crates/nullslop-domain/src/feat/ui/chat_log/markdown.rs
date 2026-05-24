//! Markdown rendering adapter for chat log entries.
//!
//! Bridges the `ratatui-markdown` crate's [`RichTextTheme`] trait to nullslop's
//! [`Theme`] struct, and provides a [`render_markdown`] helper that produces
//! `Vec<Line<'static>>` ready for the chat log renderer.

use std::sync::Arc;

use ratatui::text::Line;
use ratatui_markdown::highlight::{HighlightHooks, TreeSitterHighlighter};
use ratatui_markdown::markdown::{MarkdownRenderer, RenderHooks};
use ratatui_markdown::theme::{Generation, RichTextTheme};

use crate::feat::theme::Theme;

/// Render markdown text into styled lines for display in the chat log.
///
/// Creates a [`MarkdownRenderer`] with syntax highlighting hooks, parses the
/// markdown, and renders it using the nullslop theme. The `width` parameter
/// controls word-wrapping.
pub fn render_markdown(text: &str, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let text = text.trim();
    let md_theme = MarkdownTheme(theme);
    let renderer = MarkdownRenderer::new(width as usize)
        .with_render_hooks(highlight_hooks(width as usize, theme));
    let blocks = renderer.parse(text);
    renderer.render(&blocks, &md_theme)
}

/// Build the render hooks for syntax-highlighted code blocks.
fn highlight_hooks(max_width: usize, theme: &Theme) -> Box<dyn RenderHooks> {
    let highlighter = Arc::new(TreeSitterHighlighter::new());
    let hooks = HighlightHooks::new(highlighter, max_width).with_border_color(theme.muted_text);
    Box::new(hooks)
}

/// Thin wrapper around [`Theme`] that implements [`RichTextTheme`].
///
/// Needed because `Theme` is defined in `nullslop-theme` and `RichTextTheme`
/// is defined in `ratatui-markdown` — neither is local to this crate, so we
/// can't write a bare `impl RichTextTheme for Theme` (orphan rule).
struct MarkdownTheme<'a>(&'a Theme);

impl RichTextTheme for MarkdownTheme<'_> {
    fn generation(&self) -> Generation {
        Generation(1)
    }

    fn get_text_color(&self) -> ratatui::style::Color {
        self.0.primary_text
    }

    fn get_muted_text_color(&self) -> ratatui::style::Color {
        self.0.muted_text
    }

    fn get_primary_color(&self) -> ratatui::style::Color {
        self.0.focus_accent
    }

    fn get_popup_selected_background(&self) -> ratatui::style::Color {
        self.0.focus_accent
    }

    fn get_popup_selected_text_color(&self) -> ratatui::style::Color {
        self.0.primary_text
    }

    fn get_border_color(&self) -> ratatui::style::Color {
        self.0.border_unfocused
    }

    fn get_focused_border_color(&self) -> ratatui::style::Color {
        self.0.focus_accent
    }

    fn get_secondary_color(&self) -> ratatui::style::Color {
        self.0.success
    }

    fn get_info_color(&self) -> ratatui::style::Color {
        self.0.streaming
    }

    fn get_background_color(&self) -> ratatui::style::Color {
        self.0.user_block_bg
    }

    fn get_json_key_color(&self) -> ratatui::style::Color {
        self.0.focus_accent
    }

    fn get_json_string_color(&self) -> ratatui::style::Color {
        self.0.success
    }

    fn get_json_number_color(&self) -> ratatui::style::Color {
        self.0.warning
    }

    fn get_json_bool_color(&self) -> ratatui::style::Color {
        self.0.streaming
    }

    fn get_json_null_color(&self) -> ratatui::style::Color {
        self.0.muted_text
    }

    fn get_accent_yellow(&self) -> ratatui::style::Color {
        self.0.warning
    }
}
