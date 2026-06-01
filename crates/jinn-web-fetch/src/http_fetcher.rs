//! HTTP-based web fetcher using `reqwest`.
//!
//! Fetches web pages via plain HTTP requests without JavaScript rendering.
//! Supports HTML, text, and markdown output formats via pluggable extractors.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tracing;

use crate::{Extractor, FetchError, FetchOptions, FetchOutput, OutputFormat, WebFetcher};

/// A web fetcher that uses plain HTTP requests via `reqwest`.
///
/// Does not execute JavaScript. Suitable for static pages, APIs, and
/// content that does not require browser rendering.
///
/// Content extraction is delegated to [`Extractor`] implementations looked
/// up by [`OutputFormat`]. Formats without a registered extractor (e.g.,
/// [`OutputFormat::Html`]) return the raw response body unchanged.
pub struct HttpFetcher {
    /// The HTTP client used for requests.
    client: reqwest::Client,
    /// Extractor implementations keyed by output format.
    /// Formats not in the map (e.g., `Html`) pass through raw content.
    extractors: HashMap<OutputFormat, Arc<dyn Extractor>>,
}

impl HttpFetcher {
    /// Creates a new HTTP fetcher with the given extractor map and default client settings.
    #[must_use]
    pub fn new(extractors: HashMap<OutputFormat, Arc<dyn Extractor>>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::limited(10))
                .build()
                .unwrap_or_default(),
            extractors,
        }
    }
}

impl Default for HttpFetcher {
    fn default() -> Self {
        Self::new(HashMap::new())
    }
}

/// Validates that the URL has a supported scheme (http or https).
fn validate_url(url: &str) -> Result<(), FetchError> {
    match url::Url::parse(url) {
        Ok(parsed) => match parsed.scheme() {
            "http" | "https" => Ok(()),
            other => Err(FetchError::InvalidUrl(format!(
                "unsupported scheme: {other}"
            ))),
        },
        Err(e) => Err(FetchError::InvalidUrl(e.to_string())),
    }
}

/// Checks if a content-type indicates binary content.
fn is_binary_content_type(content_type: &str) -> bool {
    let ct = content_type.to_lowercase();
    // Allow text/*, application/json, application/xml, and common web types.
    // Reject images, audio, video, fonts, archives, etc.
    if ct.starts_with("text/")
        || ct.contains("json")
        || ct.contains("xml")
        || ct.contains("html")
        || ct.contains("javascript")
    {
        return false;
    }
    // Common binary prefixes.
    ct.starts_with("image/")
        || ct.starts_with("audio/")
        || ct.starts_with("video/")
        || ct.starts_with("font/")
        || ct.starts_with("application/octet-stream")
        || ct.starts_with("application/pdf")
        || ct.starts_with("application/zip")
}

#[async_trait]
impl WebFetcher for HttpFetcher {
    async fn fetch(&self, url: &str, options: FetchOptions) -> Result<FetchOutput, FetchError> {
        tracing::debug!(url = %url, format = ?options.format, "HttpFetcher: starting fetch");
        validate_url(url)?;

        tracing::trace!(url = %url, "HttpFetcher: sending GET request");
        let response = self.client.get(url).send().await.map_err(|e| {
            if e.is_timeout() {
                FetchError::Timeout
            } else {
                FetchError::Network
            }
        })?;

        let status = response.status().as_u16();
        let final_url = response.url().to_string();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();

        tracing::debug!(
            status,
            final_url = %final_url,
            content_type = %content_type,
            "HttpFetcher: response received"
        );

        if !response.status().is_success() {
            tracing::warn!(status, url = %final_url, "HttpFetcher: HTTP error");
            return Err(FetchError::Http {
                status,
                url: final_url,
            });
        }

        if is_binary_content_type(&content_type) {
            tracing::warn!(content_type = %content_type, "HttpFetcher: binary content detected");
            return Err(FetchError::BinaryContent { content_type });
        }

        let body = response
            .text()
            .await
            .map_err(|_reqwest_err| FetchError::Network)?;

        tracing::debug!(body_len = body.len(), "HttpFetcher: body received");

        let content = match self.extractors.get(&options.format) {
            Some(extractor) => extractor.extract(&body),
            None => body,
        };

        Ok(FetchOutput {
            content,
            url: final_url,
            status,
            content_type,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::map_err_ignore,
        reason = "test assertions"
    )]
    use super::*;

    #[rstest::rstest]
    fn validate_url_accepts_http() {
        assert!(validate_url("http://example.com").is_ok());
    }

    #[rstest::rstest]
    fn validate_url_accepts_https() {
        assert!(validate_url("https://example.com").is_ok());
    }

    #[rstest::rstest]
    fn validate_url_rejects_ftp() {
        let result = validate_url("ftp://example.com");
        assert!(matches!(result, Err(FetchError::InvalidUrl(_))));
    }

    #[rstest::rstest]
    fn validate_url_rejects_malformed() {
        let result = validate_url("not a url");
        assert!(matches!(result, Err(FetchError::InvalidUrl(_))));
    }

    #[rstest::rstest]
    fn is_binary_content_type_detects_images() {
        assert!(is_binary_content_type("image/png"));
    }

    #[rstest::rstest]
    fn is_binary_content_type_detects_pdf() {
        assert!(is_binary_content_type("application/pdf"));
    }

    #[rstest::rstest]
    fn is_binary_content_type_allows_html() {
        assert!(!is_binary_content_type("text/html; charset=utf-8"));
    }

    #[rstest::rstest]
    fn is_binary_content_type_allows_json() {
        assert!(!is_binary_content_type("application/json"));
    }

    #[rstest::rstest]
    fn http_fetcher_default_is_new() {
        let _fetcher = HttpFetcher::default();
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn fetch_returns_html_format() {
        // Given a mock HTTP server.
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body("<html><body><h1>Hello</h1></body></html>")
            .create_async()
            .await;

        let fetcher = HttpFetcher::new(HashMap::new());
        let result = fetcher
            .fetch(
                &server.url(),
                FetchOptions {
                    format: OutputFormat::Html,
                },
            )
            .await;

        mock.assert_async().await;

        let output = result.expect("fetch should succeed");
        assert_eq!(output.status, 200);
        assert!(output.content.contains("<h1>Hello</h1>"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn fetch_uses_extractor_for_registered_format() {
        // Given a mock HTTP server and a fetcher with a registered extractor.
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body("<html><body><h1>Hello</h1></body></html>")
            .create_async()
            .await;

        let fetcher = HttpFetcher::new(HashMap::from([(
            OutputFormat::Text,
            Arc::new(crate::MarkdownExtractor) as Arc<dyn crate::Extractor>,
        )]));
        let result = fetcher
            .fetch(
                &server.url(),
                FetchOptions {
                    format: OutputFormat::Text,
                },
            )
            .await;

        mock.assert_async().await;

        let output = result.expect("fetch should succeed");
        assert!(output.content.contains("Hello"));
        assert!(!output.content.contains('<'));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn fetch_produces_markdown_from_structured_html() {
        // Given a mock HTTP server serving a page with rich HTML structure.
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body("<html><body><h1>Title</h1><h2>Subtitle</h2><p>Some paragraph text.</p><ul><li>first item</li><li>second item</li></ul><a href=\"https://example.com\">click here</a></body></html>")
            .create_async()
            .await;

        let fetcher = HttpFetcher::new(HashMap::from([(
            OutputFormat::Markdown,
            Arc::new(crate::MarkdownExtractor) as Arc<dyn crate::Extractor>,
        )]));
        let result = fetcher
            .fetch(
                &server.url(),
                FetchOptions {
                    format: OutputFormat::Markdown,
                },
            )
            .await;

        mock.assert_async().await;

        // Then the output contains proper markdown equivalents.
        let output = result.expect("fetch should succeed");
        assert!(
            output.content.contains("# Title"),
            "h1 should become # Title"
        );
        assert!(
            output.content.contains("## Subtitle"),
            "h2 should become ## Subtitle"
        );
        assert!(
            output.content.contains("Some paragraph text"),
            "paragraph text should be preserved"
        );
        assert!(
            output.content.contains("first item"),
            "list items should be preserved"
        );
        assert!(
            output.content.contains("[click here](https://example.com)"),
            "link should become markdown link"
        );
        assert!(
            !output.content.contains('<'),
            "no raw HTML tags should remain"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn fetch_returns_raw_body_for_unregistered_format() {
        // Given a mock HTTP server and a fetcher with no extractors.
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body("<html><body><h1>Hello</h1></body></html>")
            .create_async()
            .await;

        let fetcher = HttpFetcher::new(HashMap::new());
        let result = fetcher
            .fetch(
                &server.url(),
                FetchOptions {
                    format: OutputFormat::Text,
                },
            )
            .await;

        mock.assert_async().await;

        let output = result.expect("fetch should succeed");
        assert!(output.content.contains("<h1>Hello</h1>"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn fetch_returns_error_for_invalid_url() {
        let fetcher = HttpFetcher::new(HashMap::new());
        let result = fetcher.fetch("not-a-url", FetchOptions::default()).await;

        assert!(matches!(result, Err(FetchError::InvalidUrl(_))));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn fetch_returns_error_for_http_error_status() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/")
            .with_status(404)
            .create_async()
            .await;

        let fetcher = HttpFetcher::new(HashMap::new());
        let result = fetcher.fetch(&server.url(), FetchOptions::default()).await;

        mock.assert_async().await;

        assert!(matches!(result, Err(FetchError::Http { status: 404, .. })));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn fetch_returns_error_for_binary_content() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "image/png")
            .with_body("binary data")
            .create_async()
            .await;

        let fetcher = HttpFetcher::new(HashMap::new());
        let result = fetcher.fetch(&server.url(), FetchOptions::default()).await;

        mock.assert_async().await;

        assert!(matches!(result, Err(FetchError::BinaryContent { .. })));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn fetch_reports_final_url_after_redirect() {
        let mut server = mockito::Server::new_async().await;
        let target = format!("{}/final", server.url());
        let mock_redirect = server
            .mock("GET", "/redirect")
            .with_status(302)
            .with_header("location", &target)
            .create_async()
            .await;
        let mock_final = server
            .mock("GET", "/final")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("final page")
            .create_async()
            .await;

        let fetcher = HttpFetcher::new(HashMap::new());
        let result = fetcher
            .fetch(
                &format!("{}/redirect", server.url()),
                FetchOptions::default(),
            )
            .await;

        mock_redirect.assert_async().await;
        mock_final.assert_async().await;

        let output = result.expect("fetch should succeed");
        assert!(output.url.ends_with("/final"));
    }
}
