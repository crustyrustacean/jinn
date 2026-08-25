//! Web search abstraction — trait, types, and error definitions.
//!
//! Defines the [`WebSearcher`] trait for running web searches and collecting
//! results. The DuckDuckGo HTML implementation lives in [`ddg_searcher`].
//!
//! This crate is a leaf dependency (like `jinn-web-fetch`): it knows nothing
//! about jinn's actor system. Actors in `jinn-domain` own an `Arc<dyn
//! WebSearcher>` and call [`WebSearcher::search`].

pub mod browser_ddg_searcher;
pub mod ddg_searcher;
pub mod html_parser;

pub use browser_ddg_searcher::BrowserDdgSearcher;
pub use ddg_searcher::DdgSearcher;

use async_trait::async_trait;
use wherror::Error;

/// One search result: a titled link with a short snippet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    /// The result's title (visible link text).
    pub title: String,
    /// The decoded, final URL the result points to.
    pub url: String,
    /// A short text snippet describing the result.
    pub snippet: String,
}

/// Options for a search request.
///
/// Carried from tool configuration; the caller may also override `max_results`
/// per-call (the actor applies the lower of config and the per-call value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOptions {
    /// Maximum number of results to return.
    pub max_results: usize,
    /// DuckDuckGo region code, e.g. `"wt-wt"` (global) or `"us-en"`.
    pub region: String,
    /// Whether safe search is on. `true` → `kp=1`, `false` → `kp=-2`.
    pub safe_search: bool,
}

/// Errors that can occur during a web search.
#[derive(Debug, Error)]
pub enum SearchError {
    /// The query was empty or otherwise invalid.
    #[error("invalid query")]
    InvalidQuery,
    /// A network-level error occurred (DNS, connection refused, TLS, etc.).
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
    /// DuckDuckGo returned an anti-bot / unusual-traffic challenge page that
    /// did not clear. In headless mode (or after the human-solve window
    /// expired), switching the browser-backed backends to `headed-chrome`
    /// lets a human pass the challenge in the visible tab.
    #[error(
        "DuckDuckGo blocked the request (anti-bot challenge) — switch [web_search]/[web_fetch] browser backends to headed-chrome to solve it manually"
    )]
    Blocked,
}

/// Async web searcher.
///
/// Implementations (e.g. [`DdgSearcher`]) run a search and return decoded
/// results. The actor layer wraps this trait so tests can substitute a mock.
#[async_trait]
pub trait WebSearcher: Send + Sync {
    /// Run a search for `query` with the given `options`, returning results.
    ///
    /// # Errors
    ///
    /// Returns [`SearchError`] on network failure, non-success HTTP status,
    /// an anti-bot block, or an invalid query.
    async fn search(
        &self,
        query: &str,
        options: &SearchOptions,
    ) -> Result<Vec<SearchResult>, SearchError>;

    /// Runs a search with a progress observer for long waits (challenge
    /// solving). The default ignores the observer and delegates to
    /// [`Self::search`] — only browser-backed implementations that can wait
    /// on a human need to override it.
    ///
    /// # Errors
    ///
    /// Same as [`Self::search`].
    async fn search_observed(
        &self,
        query: &str,
        options: &SearchOptions,
        on_event: jinn_web_fetch::ProgressFn,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let _ = on_event;
        self.search(query, options).await
    }
}
