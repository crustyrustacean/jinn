//! Content extraction from HTML pages.
//!
//! Defines the [`Extractor`] trait for converting raw HTML into alternative
//! formats. Implementations are registered in the fetcher's extractor map and
//! selected at fetch time based on the requested [`OutputFormat`](crate::OutputFormat).
//!
//! # Extensibility
//!
//! To add a new extraction mode:
//! 1. Implement [`Extractor`] for your strategy.
//! 2. Register it in the extractor map at fetcher construction time.

/// Extracts structured content from raw HTML.
///
/// Implementations define a specific extraction strategy (e.g., markdown,
/// plain text, readability). The trait is format-agnostic - callers select
/// which extractor to use based on the desired output format.
pub trait Extractor: Send + Sync {
    /// Extracts content from the given HTML string.
    fn extract(&self, html: &str) -> String;
}

/// HTML element tags dropped entirely (with their subtrees) by the
/// [`CleanMarkdownExtractor`].
///
/// These are boilerplate / non-content elements whose text adds noise without
/// information: executable `<script>`/`<style>`, alternate `<noscript>` content,
/// `<footer>`/`<aside>` chrome, interactive `<form>`, and structural
/// `<svg>`/`<math>`/`<iframe>`/`<template>`.
///
/// **Kept on purpose:** `<nav>` and `<header>`. Many sites nest their primary
/// navigation inside `<header>`, and `htmd`'s `skip_tags` drops the *entire*
/// subtree — skipping `<header>` would also delete the nav links. Keeping them
/// preserves link discovery for agents.
const SKIP_TAGS: &[&str] = &[
    "script", "style", "noscript", "aside", "footer", "form", "svg", "math", "iframe", "template",
];

/// Extracts markdown from HTML using the `htmd` library.
///
/// Converts HTML elements to their markdown equivalents (e.g., `<h1>` → `#`,
/// `<a>` → `[text](url)`, `<ul><li>` → `- item`). Falls back to an empty
/// string if conversion fails.
pub struct MarkdownExtractor;

impl Extractor for MarkdownExtractor {
    fn extract(&self, html: &str) -> String {
        htmd::convert(html).unwrap_or_default()
    }
}

/// Extracts boilerplate-stripped markdown from HTML.
///
/// Like [`MarkdownExtractor`] but drops the subtrees listed in [`SKIP_TAGS`]
/// (script, style, footer, aside, form, svg, math, iframe, template, noscript)
/// before conversion, via `htmd`'s native `skip_tags`. Keeps `<nav>` and
/// `<header>` so links stay discoverable. Falls back to an empty string if
/// conversion fails.
pub struct CleanMarkdownExtractor;

impl Extractor for CleanMarkdownExtractor {
    fn extract(&self, html: &str) -> String {
        htmd::HtmlToMarkdown::builder()
            .skip_tags(SKIP_TAGS.to_vec())
            .build()
            .convert(html)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "test assertions"
    )]

    use super::*;

    #[rstest::rstest]
    fn extract_produces_markdown_headers() {
        // Given a MarkdownExtractor and HTML with headers.
        let extractor = MarkdownExtractor;

        // When extracting content.
        let result = extractor.extract("<h1>Title</h1><h2>Subtitle</h2>");

        // Then the output contains markdown headers.
        assert!(result.contains("# Title"), "should contain h1 as # Title");
        assert!(
            result.contains("## Subtitle"),
            "should contain h2 as ## Subtitle"
        );
    }

    #[rstest::rstest]
    fn extract_produces_markdown_links() {
        // Given a MarkdownExtractor and HTML with a link.
        let extractor = MarkdownExtractor;

        // When extracting content.
        let result = extractor.extract(r#"<a href="https://example.com">click</a>"#);

        // Then the output contains a markdown link.
        assert!(
            result.contains("[click](https://example.com)"),
            "should contain markdown link syntax"
        );
    }

    #[rstest::rstest]
    fn extract_produces_markdown_lists() {
        // Given a MarkdownExtractor and HTML with a list.
        let extractor = MarkdownExtractor;

        // When extracting content.
        let result = extractor.extract("<ul><li>first</li><li>second</li></ul>");

        // Then the output contains the list item text.
        assert!(
            result.contains("first"),
            "should contain first list item text"
        );
        assert!(
            result.contains("second"),
            "should contain second list item text"
        );
    }

    #[rstest::rstest]
    fn extract_produces_markdown_code_blocks() {
        // Given a MarkdownExtractor and HTML with a code block.
        let extractor = MarkdownExtractor;

        // When extracting content.
        let result = extractor.extract("<pre><code>let x = 1;</code></pre>");

        // Then the output contains a fenced code block.
        assert!(
            result.contains("let x = 1;"),
            "should preserve code content"
        );
    }

    #[rstest::rstest]
    fn extract_handles_empty_html() {
        // Given a MarkdownExtractor and empty input.
        let extractor = MarkdownExtractor;

        // When extracting content.
        let result = extractor.extract("");

        // Then the output is empty.
        assert!(result.is_empty(), "empty input should produce empty output");
    }

    #[rstest::rstest]
    fn extract_produces_markdown_paragraphs() {
        // Given a MarkdownExtractor and HTML with paragraphs.
        let extractor = MarkdownExtractor;

        // When extracting content.
        let result = extractor.extract("<p>Hello</p><p>World</p>");

        // Then the output contains both paragraph texts.
        assert!(result.contains("Hello"), "should contain first paragraph");
        assert!(result.contains("World"), "should contain second paragraph");
    }

    // ---- CleanMarkdownExtractor ----

    #[rstest::rstest]
    #[test]
    fn clean_extractor_drops_inline_script() {
        // Given a CleanMarkdownExtractor and HTML with an inline script.
        let extractor = CleanMarkdownExtractor;

        // When extracting content.
        let result = extractor.extract("<p>hello</p><script>alert(1)</script>");

        // Then the script body is absent and the paragraph survives.
        assert!(result.contains("hello"), "content must survive");
        assert!(!result.contains("alert"), "script body must be dropped");
    }

    #[rstest::rstest]
    #[test]
    fn clean_extractor_drops_style() {
        // Given a CleanMarkdownExtractor and HTML with a style block.
        let extractor = CleanMarkdownExtractor;

        // When extracting content.
        let result = extractor.extract("<style>body { color: red; }</style><p>hello</p>");

        // Then the CSS is absent and the paragraph survives.
        assert!(result.contains("hello"), "content must survive");
        assert!(!result.contains("color: red"), "css must be dropped");
    }

    #[rstest::rstest]
    #[test]
    fn clean_extractor_drops_aside() {
        // Given a CleanMarkdownExtractor and HTML with an aside (sidebar).
        let extractor = CleanMarkdownExtractor;

        // When extracting content.
        let result = extractor.extract("<aside>related posts</aside><p>main</p>");

        // Then the sidebar text is absent and main content survives.
        assert!(result.contains("main"), "main content must survive");
        assert!(!result.contains("related"), "aside must be dropped");
    }

    #[rstest::rstest]
    #[test]
    fn clean_extractor_drops_footer() {
        // Given a CleanMarkdownExtractor and HTML with a footer.
        let extractor = CleanMarkdownExtractor;

        // When extracting content.
        let result = extractor.extract("<p>main</p><footer>© 2026 Acme</footer>");

        // Then the footer text is absent and main content survives.
        assert!(result.contains("main"), "main content must survive");
        assert!(!result.contains("Acme"), "footer must be dropped");
    }

    #[rstest::rstest]
    #[test]
    fn clean_extractor_drops_form() {
        // Given a CleanMarkdownExtractor and HTML with a form.
        let extractor = CleanMarkdownExtractor;

        // When extracting content.
        let result = extractor.extract("<form><input placeholder=\"query\"></form><p>main</p>");

        // Then the form widget is absent and main content survives.
        assert!(result.contains("main"), "main content must survive");
        assert!(!result.contains("query"), "form must be dropped");
    }

    #[rstest::rstest]
    #[test]
    fn clean_extractor_drops_svg() {
        // Given a CleanMarkdownExtractor and HTML with inline SVG.
        let extractor = CleanMarkdownExtractor;

        // When extracting content.
        let result = extractor.extract("<svg><circle/></svg><p>main</p>");

        // Then the SVG markup is absent and main content survives.
        assert!(result.contains("main"), "main content must survive");
        assert!(!result.contains("circle"), "svg must be dropped");
    }

    #[rstest::rstest]
    #[test]
    fn clean_extractor_drops_math() {
        // Given a CleanMarkdownExtractor and HTML with MathML.
        let extractor = CleanMarkdownExtractor;

        // When extracting content.
        let result = extractor.extract("<math><mi>quadratic</mi></math><p>main</p>");

        // Then the MathML content is absent and main content survives.
        assert!(result.contains("main"), "main content must survive");
        assert!(
            !result.contains("quadratic"),
            "math content must be dropped"
        );
    }

    #[rstest::rstest]
    #[test]
    fn clean_extractor_keeps_nav_links() {
        // Given a CleanMarkdownExtractor and HTML with a nav containing links.
        let extractor = CleanMarkdownExtractor;

        // When extracting content.
        let result = extractor.extract(r#"<nav><a href="/docs">Docs</a></nav><p>main</p>"#);

        // Then both the link text and href survive.
        assert!(result.contains("Docs"), "nav link text must survive");
        assert!(result.contains("/docs"), "nav link href must survive");
    }

    #[rstest::rstest]
    #[test]
    fn clean_extractor_keeps_header() {
        // Given a CleanMarkdownExtractor and HTML with a header.
        let extractor = CleanMarkdownExtractor;

        // When extracting content.
        let result = extractor.extract("<header><h1>Site Title</h1></header><p>main</p>");

        // Then the header content survives (so nested nav links would too).
        assert!(result.contains("Site Title"), "header content must survive");
    }

    #[rstest::rstest]
    #[test]
    fn clean_extractor_converts_article_to_markdown() {
        // Given a CleanMarkdownExtractor and HTML with an article.
        let extractor = CleanMarkdownExtractor;

        // When extracting content.
        let result = extractor.extract("<article><h1>Title</h1><p>body</p></article>");

        // Then content is converted to markdown.
        assert!(
            result.contains("# Title"),
            "h1 must become a markdown heading"
        );
        assert!(result.contains("body"), "paragraph must survive");
    }

    #[rstest::rstest]
    #[test]
    fn clean_extractor_returns_empty_for_empty_html() {
        // Given a CleanMarkdownExtractor and empty input.
        let extractor = CleanMarkdownExtractor;

        // When extracting content.
        let result = extractor.extract("");

        // Then the output is empty.
        assert!(result.is_empty(), "empty input should produce empty output");
    }
}
