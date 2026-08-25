//! DuckDuckGo HTML endpoint searcher.
//!
//! [`DdgSearcher`] POSTs a form to DuckDuckGo's HTML results page
//! (`https://html.duckduckgo.com/html`), detects anti-bot challenges, and
//! extracts results via [`crate::html_parser`]. The form fields mirror ddgr's
//! page-0 request (the only page we fetch — there is no pagination).

use async_trait::async_trait;

use crate::{SearchError, SearchOptions, SearchResult, WebSearcher, html_parser};

/// The fallback Chrome User-Agent. DDG blocks requests without a browser UA.
///
/// Wiring should inject the UA resolved from `[browser]` config (matching the
/// detected binary) via [`DdgSearcher::with_user_agent`]; this constant is only
/// the fallback when no UA is supplied.
const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// Phrases that appear in DDG's anti-bot / unusual-traffic challenge page.
/// [`DdgSearcher::is_blocked`] delegates to the shared marker table in
/// `jinn_web_fetch::challenge` so both backends detect the same page.
/// DuckDuckGo HTML searcher.
///
/// Owns a `reqwest::Client` (connection-pooled) and the endpoint base URL. The
/// base URL defaults to the real DDG endpoint; tests override it to point at a
/// `mockito` server.
pub struct DdgSearcher {
    client: reqwest::Client,
    /// The DDG HTML endpoint to POST to, without trailing slash.
    /// Default: `https://html.duckduckgo.com/html`.
    endpoint: String,
}

impl DdgSearcher {
    /// Creates a searcher targeting the real DuckDuckGo endpoint with the
    /// default UA.
    ///
    /// Prefer [`Self::with_user_agent`] in wiring so the UA matches the
    /// detected browser binary rather than the stale default.
    ///
    /// # Errors
    ///
    /// Returns [`SearchError::Network`] only if the HTTP client cannot be built
    /// (e.g. a TLS backend failure). This is extraordinarily rare.
    #[must_use]
    pub fn new() -> Self {
        Self::with_endpoint("https://html.duckduckgo.com/html".to_owned())
    }

    /// Creates a searcher targeting the real DuckDuckGo endpoint with an
    /// injected user agent. Use this in wiring when the UA is resolved from
    /// the `[browser]` config so search over HTTP sends a current, realistic
    /// UA instead of the stale default.
    #[must_use]
    pub fn with_user_agent(user_agent: &str) -> Self {
        Self::with_endpoint_and_user_agent(
            "https://html.duckduckgo.com/html".to_owned(),
            user_agent,
        )
    }

    /// Creates a searcher targeting a custom endpoint (used by tests to point
    /// at a `mockito` server).
    #[must_use]
    pub fn with_endpoint(endpoint: String) -> Self {
        Self::with_endpoint_and_user_agent(endpoint, DEFAULT_USER_AGENT)
    }

    /// Creates a searcher targeting a custom endpoint with an injected UA.
    #[must_use]
    pub fn with_endpoint_and_user_agent(endpoint: String, user_agent: &str) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(user_agent)
            .redirect(reqwest::redirect::Policy::limited(10))
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default();
        Self { client, endpoint }
    }

    /// Builds the DDG page-0 form body for a query + options.
    ///
    /// Returns an ordered list of `(key, value)` pairs. This is extracted into a
    /// method so the form-contents test can assert against it without a network
    /// round-trip.
    pub(crate) fn form_fields<'a>(
        query: &'a str,
        options: &'a SearchOptions,
    ) -> Vec<(&'static str, std::borrow::Cow<'a, str>)> {
        use std::borrow::Cow;
        // kp: safe search value. 1 = on (default), -2 = off.
        let kp = if options.safe_search { "1" } else { "-2" };
        vec![
            ("q", Cow::Borrowed(query)),
            ("b", Cow::Borrowed("")),
            ("df", Cow::Borrowed("")),
            ("kf", Cow::Borrowed("-1")),
            ("kh", Cow::Borrowed("1")),
            ("kl", Cow::Borrowed(options.region.as_str())),
            ("kp", Cow::Borrowed(kp)),
            ("k1", Cow::Borrowed("-1")),
        ]
    }

    /// Returns `true` if the response body looks like an anti-bot challenge.
    pub(crate) fn is_blocked(body: &str) -> bool {
        jinn_web_fetch::challenge::is_ddg_blocked(body)
    }
}

impl Default for DdgSearcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WebSearcher for DdgSearcher {
    async fn search(
        &self,
        query: &str,
        options: &SearchOptions,
    ) -> Result<Vec<SearchResult>, SearchError> {
        if query.trim().is_empty() {
            return Err(SearchError::InvalidQuery);
        }

        let form = Self::form_fields(query, options);
        let response = self
            .client
            .post(&self.endpoint)
            .header("DNT", "1")
            .header(
                reqwest::header::ACCEPT_ENCODING,
                reqwest::header::HeaderValue::from_static("gzip"),
            )
            .form(&form)
            .send()
            .await
            .map_err(|err| map_send_error(&err))?;

        let status = response.status();
        if !status.is_success() {
            return Err(SearchError::Http {
                status: status.as_u16(),
                url: self.endpoint.clone(),
            });
        }

        let body = response
            .text()
            .await
            .map_err(|_body_err| SearchError::Network)?;

        if Self::is_blocked(&body) {
            return Err(SearchError::Blocked);
        }

        let results = html_parser::parse_results(&body, options.max_results);
        Ok(results)
    }
}

/// Maps a reqwest send error to a [`SearchError`].
///
/// reqwest collapses timeouts and connect failures into `reqwest::Error`; we
/// classify by `is_timeout()` / `is_connect()` when available.
fn map_send_error(err: &reqwest::Error) -> SearchError {
    if err.is_timeout() {
        SearchError::Timeout
    } else {
        SearchError::Network
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

    const FIXTURE: &str = include_str!("../tests/fixtures/ddg_search_results.html");
    const BLOCKED_FIXTURE: &str = include_str!("../tests/fixtures/ddg_blocked.html");

    fn opts() -> SearchOptions {
        SearchOptions {
            max_results: 10,
            region: "wt-wt".to_owned(),
            safe_search: true,
        }
    }

    /// Collects the form-field Cow pairs into an owned `(String, String)` map
    /// keyed by field name, for easy assertion in the form-field tests.
    fn field_map<'a, I>(pairs: I) -> std::collections::HashMap<String, String>
    where
        I: IntoIterator<Item = (&'static str, std::borrow::Cow<'a, str>)>,
    {
        pairs
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.into_owned()))
            .collect()
    }

    #[test]
    fn form_fields_contain_query_and_region() {
        // Given a query and options.
        let options = opts();

        // When building the form.
        let form = field_map(DdgSearcher::form_fields("rust lang", &options));

        // Then the query, region, safe-search, and ads-off fields are present.
        assert_eq!(form.get("q").map(String::as_str), Some("rust lang"));
        assert_eq!(form.get("kl").map(String::as_str), Some("wt-wt"));
        assert_eq!(form.get("kp").map(String::as_str), Some("1"));
        assert_eq!(form.get("k1").map(String::as_str), Some("-1"));
        // The required empty `b` field is present.
        assert_eq!(form.get("b").map(String::as_str), Some(""));
    }

    #[test]
    fn form_fields_safe_search_off_sets_kp_minus_two() {
        // Given safe search off.
        let options = SearchOptions {
            safe_search: false,
            ..opts()
        };

        // When building the form.
        let form = field_map(DdgSearcher::form_fields("q", &options));

        // Then kp is -2 (safe search off).
        assert_eq!(form.get("kp").map(String::as_str), Some("-2"));
    }

    #[test]
    fn is_blocked_detects_antibot_markers() {
        // Given the blocked fixture.
        // When checking.
        let blocked = DdgSearcher::is_blocked(BLOCKED_FIXTURE);

        // Then it is detected as a block.
        assert!(blocked, "blocked fixture must be detected");
    }

    #[test]
    fn is_blocked_returns_false_for_real_results() {
        // Given the results fixture.
        // When checking.
        let blocked = DdgSearcher::is_blocked(FIXTURE);

        // Then it is NOT a block.
        assert!(!blocked, "results fixture must not be flagged as blocked");
    }

    #[tokio::test]
    async fn with_user_agent_sends_injected_ua_header() {
        // Given a mock server that matches on the injected UA header.
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/html")
            .match_header("user-agent", "MyCustomUA/9.9")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body("<html><body></body></html>")
            .create_async()
            .await;
        let searcher = DdgSearcher::with_endpoint_and_user_agent(
            format!("{}/html", server.url()),
            "MyCustomUA/9.9",
        );

        // When searching.
        let results = searcher.search("rust", &opts()).await.expect("ok");

        // Then the request matched (the injected UA was sent) and parsed empty.
        assert!(results.is_empty());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn search_returns_error_for_empty_query() {
        // Given a searcher.
        let searcher = DdgSearcher::new();

        // When searching with an empty query (no server needed — fails first).
        let result = searcher.search("   ", &opts()).await;

        // Then InvalidQuery is returned.
        assert!(matches!(result, Err(SearchError::InvalidQuery)));
    }

    #[tokio::test]
    async fn search_returns_error_on_http_failure() {
        // Given a mock server that returns 503.
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/html")
            .with_status(503)
            .create_async()
            .await;
        let searcher = DdgSearcher::with_endpoint(format!("{}/html", server.url()));

        // When searching.
        let result = searcher.search("rust", &opts()).await;

        // Then an HTTP error is returned.
        assert!(
            matches!(result, Err(SearchError::Http { status, .. }) if status == 503),
            "expected Http(503), got {result:?}"
        );
    }

    #[tokio::test]
    async fn search_returns_blocked_when_challenge_page_returned() {
        // Given a mock server returning the anti-bot page with 200.
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/html")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body(BLOCKED_FIXTURE)
            .create_async()
            .await;
        let searcher = DdgSearcher::with_endpoint(format!("{}/html", server.url()));

        // When searching.
        let result = searcher.search("rust", &opts()).await;

        // Then Blocked is returned.
        assert!(
            matches!(result, Err(SearchError::Blocked)),
            "expected Blocked, got {result:?}"
        );
    }

    #[tokio::test]
    async fn search_parses_results_from_successful_response() {
        // Given a mock server returning the results fixture.
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/html")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body(FIXTURE)
            .create_async()
            .await;
        let searcher = DdgSearcher::with_endpoint(format!("{}/html", server.url()));

        // When searching.
        let results = searcher
            .search("rust programming", &opts())
            .await
            .expect("ok");

        // Then results are parsed (the fixture has 5 valid results).
        assert_eq!(results.len(), 5);
        assert_eq!(results[0].title, "Rust Programming Language");
    }

    #[tokio::test]
    async fn search_truncates_to_max_results() {
        // Given a mock server returning the 5-result fixture.
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/html")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body(FIXTURE)
            .create_async()
            .await;
        let searcher = DdgSearcher::with_endpoint(format!("{}/html", server.url()));

        // When searching with max_results = 2.
        let options = SearchOptions {
            max_results: 2,
            ..opts()
        };
        let results = searcher.search("rust", &options).await.expect("ok");

        // Then exactly 2 results are returned.
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn search_returns_empty_for_page_with_no_results() {
        // Given a mock server returning valid HTML with no results.
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/html")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body("<html><body><p>nothing here</p></body></html>")
            .create_async()
            .await;
        let searcher = DdgSearcher::with_endpoint(format!("{}/html", server.url()));

        // When searching.
        let results = searcher.search("obscure", &opts()).await.expect("ok");

        // Then an empty vec is returned (not an error).
        assert!(results.is_empty());
    }
}
