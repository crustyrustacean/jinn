//! Markdown rendering adapter for chat log entries.
//!
//! Bridges the `ratatui-markdown` crate's [`RichTextTheme`] trait to jinn's
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
/// markdown, and renders it using the jinn theme. The `width` parameter
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
/// Needed because `Theme` is defined in `jinn-theme` and `RichTextTheme`
/// is defined in `ratatui-markdown` - neither is local to this crate, so we
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

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unwrap_used,
        reason = "test code"
    )]

    use ratatui_markdown::highlight::CodeHighlighter;

    use super::*;

    const WIDTH: u16 = 80;

    /// Render a fenced code block and return the rendered lines.
    fn render_code_block(lang: &str, code: &str, theme: &Theme) -> Vec<Line<'static>> {
        let markdown = format!("```{lang}\n{code}\n```");
        render_markdown(&markdown, WIDTH, theme)
    }

    /// Spans carrying code text — i.e. not the box-drawing header/footer/prefix
    /// spans (`╭`, `╰`, `│`) that every code block gets regardless of language.
    fn code_text_spans<'a>(lines: &'a [Line<'static>]) -> Vec<&'a ratatui::text::Span<'static>> {
        lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .filter(|s| !s.content.starts_with(['╭', '╰', '│']))
            .collect()
    }

    /// The style the plain (non-highlighted) render path gives code text.
    fn plain_code_style(theme: &Theme) -> ratatui::style::Style {
        ratatui::style::Style::default().fg(theme.warning)
    }

    #[rstest::rstest]
    #[test]
    fn curated_language_fenced_block_receives_highlight_styling() {
        // Given a python fenced block (a curated grammar).
        let theme = crate::feat::theme::default_theme();

        // When rendering.
        let lines = render_code_block(
            "python",
            "def greet(name):\n    return f\"hi {name}\"",
            &theme,
        );

        // Then some code text is styled beyond the plain-path color — the
        // highlighter engaged.
        let plain = plain_code_style(&theme);
        assert!(
            code_text_spans(&lines).iter().any(|s| s.style != plain),
            "python block should be highlighted"
        );
    }

    #[rstest::rstest]
    #[test]
    fn excluded_language_fenced_block_renders_plain_without_panic() {
        // Given an ocaml fenced block (a grammar not in the curated set).
        let theme = crate::feat::theme::default_theme();

        // When rendering.
        let lines = render_code_block("ocaml", "let x = 1 in print_int x", &theme);

        // Then every code text span carries exactly the plain-path style —
        // no highlight spans leaked in, and no panic.
        let plain = plain_code_style(&theme);
        assert!(
            code_text_spans(&lines).iter().all(|s| s.style == plain),
            "ocaml block should render plain"
        );
        // And the code text is still present.
        let joined: String = code_text_spans(&lines)
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(joined.contains("print_int"), "code text must survive");
    }

    #[rstest::rstest]
    #[case("rust", "let x = 1;")]
    #[case("python", "x = 1")]
    #[case("py", "x = 1")]
    #[case("javascript", "let x = 1;")]
    #[case("js", "let x = 1;")]
    #[case("typescript", "let x: number = 1;")]
    #[case("ts", "let x: number = 1;")]
    #[case("tsx", "const x = <div />;")]
    #[case("bash", "echo hi")]
    #[case("sh", "echo hi")]
    #[case("json", r#"{ "k": 1 }"#)]
    #[case("toml", "k = 1")]
    #[case("go", "var x int = 1")]
    #[case("golang", "var x int = 1")]
    #[case("c", "int x = 1;")]
    #[case("cpp", "int x = 1;")]
    fn curated_language_produces_highlight_segments(#[case] lang: &str, #[case] code: &str) {
        // Given the curated grammar set (see root Cargo.toml features).

        // When highlighting a snippet in a curated language (or alias).
        let segments = TreeSitterHighlighter::new().highlight(lang, code);

        // Then highlight segments are produced.
        assert!(!segments.is_empty(), "{lang} should highlight");
    }

    #[rstest::rstest]
    #[case("ocaml")]
    #[case("ruby")]
    #[case("java")]
    #[case("haskell")]
    #[case("zig")]
    #[case("lua")]
    #[case("sql")]
    #[case("yaml")]
    #[case("html")]
    #[case("csharp")]
    fn excluded_language_produces_no_highlight_segments(#[case] lang: &str) {
        // Given the curated grammar set, which omits these languages.

        // When highlighting a snippet in an excluded language.
        let segments = TreeSitterHighlighter::new().highlight(lang, "let x = 1;");

        // Then no segments come back — the grammar was not compiled in.
        assert!(segments.is_empty(), "{lang} should not highlight");
    }
}
