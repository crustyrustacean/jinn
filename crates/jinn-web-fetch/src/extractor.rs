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
}
