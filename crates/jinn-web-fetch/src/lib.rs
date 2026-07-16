//! Web page fetching abstraction - trait, types, and error definitions.
//!
//! Defines the [`WebFetcher`] trait for fetching web page content with
//! multiple output formats. Implementations are provided by separate modules
//! (e.g., [`HttpFetcher`], `HeadlessChromeFetcher`).

pub mod challenge;
pub mod extractor;
pub mod http_fetcher;
pub mod stealth;

#[cfg(feature = "headless-chrome")]
pub mod shared_browser;
#[cfg(feature = "headless-chrome")]
pub mod headless_chrome_fetcher;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use wherror::Error;

pub use extractor::{CleanMarkdownExtractor, Extractor, MarkdownExtractor};
pub use http_fetcher::HttpFetcher;

#[cfg(feature = "headless-chrome")]
pub use headless_chrome_fetcher::HeadlessChromeFetcher;
#[cfg(feature = "headless-chrome")]
pub use shared_browser::{ChromeBrowser, ChromeFactory, HeadlessBrowser, HeadlessBrowserFactory, RenderedPage, SharedBrowser};

/// Output format for fetched web page content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum OutputFormat {
    /// Raw HTML source.
    Html,
    /// HTML converted to Markdown, with no boilerplate stripping.
    ///
    /// Includes the full page: nav, header, footer, and everything.
    Markdown,
    /// HTML converted to Markdown with boilerplate stripped.
    ///
    /// Drops script, style, and structural chrome (footer, aside, form,
    /// svg, math, iframe, template, noscript) while keeping nav and header
    /// so links remain discoverable. The default output format.
    #[default]
    MarkdownClean,
}

/// Options for a web fetch request.
///
/// Extensible struct - new fields (headers, cookies, timeouts) can be
/// added here without breaking the trait signature.
#[derive(Debug, Clone, Default)]
pub struct FetchOptions {
    /// The desired output format. Defaults to [`OutputFormat::MarkdownClean`].
    pub format: OutputFormat,
}

/// The result of a successful web fetch.
#[derive(Debug, Clone)]
pub struct FetchOutput {
    /// The page content in the requested format.
    pub content: String,
    /// The final URL after following redirects.
    pub url: String,
    /// The HTTP status code of the response.
    pub status: u16,
    /// The Content-Type header value of the response.
    pub content_type: String,
}

/// Errors that can occur during web fetching.
#[derive(Debug, Error)]
pub enum FetchError {
    /// The URL is malformed or uses an unsupported scheme.
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    /// A network-level error occurred (DNS, connection refused, etc.).
    #[error("network error")]
    Network,
    /// The request timed out.
    #[error("request timed out")]
    Timeout,
    /// The server returned a non-success HTTP status.
    #[error("HTTP error {status}: {url}")]
    Http {
        /// The HTTP status code.
        status: u16,
        /// The URL that was requested.
        url: String,
    },
    /// The response contains binary content that cannot be represented as text.
    #[error("binary content ({content_type})")]
    BinaryContent {
        /// The Content-Type of the response.
        content_type: String,
    },
    /// The browser failed to launch.
    #[error("browser launch failed")]
    BrowserLaunch,
    /// The browser crashed or became unresponsive.
    #[error("browser crashed")]
    BrowserCrash,
    /// An error occurred while rendering or extracting page content.
    #[error("render error: {0}")]
    Render(String),
}

/// Trait for fetching web page content.
///
/// Implementations provide different fetching strategies (plain HTTP,
/// headless browser, etc.). The trait is async and `Send + Sync` for
/// use in actor systems.
#[async_trait]
pub trait WebFetcher: Send + Sync {
    /// Fetches a web page and returns its content in the requested format.
    ///
    /// # Errors
    ///
    /// Returns [`FetchError`] if the URL is invalid, the network request
    /// fails, or content extraction fails.
    async fn fetch(&self, url: &str, options: FetchOptions) -> Result<FetchOutput, FetchError>;

    /// Shuts down the fetcher and releases any held resources.
    ///
    /// Called during application shutdown. The default implementation is
    /// a no-op for stateless implementations.
    async fn shutdown(&self) {}
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test assertions")]

    use super::OutputFormat;

    #[rstest::rstest]
    #[case(OutputFormat::Html, "html")]
    #[case(OutputFormat::Markdown, "markdown")]
    #[case(OutputFormat::MarkdownClean, "markdown-clean")]
    fn output_format_serializes_to_kebab_case(
        #[case] format: OutputFormat,
        #[case] expected: &str,
    ) {
        // Given an OutputFormat variant.
        // When serializing.
        let serialized = serde_json::to_string(&format).expect("serialize");

        // Then it matches the expected kebab-case string.
        assert_eq!(serialized, format!("\"{expected}\""));
    }

    #[rstest::rstest]
    #[case("html", OutputFormat::Html)]
    #[case("markdown", OutputFormat::Markdown)]
    #[case("markdown-clean", OutputFormat::MarkdownClean)]
    fn output_format_deserializes_from_kebab_case(#[case] s: &str, #[case] expected: OutputFormat) {
        // Given a kebab-case format string.
        // When deserializing.
        let deserialized: OutputFormat =
            serde_json::from_str(&format!("\"{s}\"")).expect("deserialize");

        // Then it matches the expected variant.
        assert_eq!(deserialized, expected);
    }

    #[test]
    fn output_format_default_is_markdown_clean() {
        // Given no explicit format.
        // When using the default.
        let default = OutputFormat::default();

        // Then it is MarkdownClean.
        assert_eq!(default, OutputFormat::MarkdownClean);
    }
}
