//! DuckDuckGo browser-backed searcher.
//!
//! [`BrowserDdgSearcher`] drives the shared browser (`jinn_web_fetch::SharedBrowser`)
//! to render DuckDuckGo's HTML results page. This is the anti-bot-hardened search
//! path: a real browser (optionally headed with a warmed persistent profile) clears
//! the JS/Turnstile challenges that block the plain [`crate::DdgSearcher`] HTTP path.
//!
//! The browser renders the DDG **GET** endpoint (`/html/?q=...`) rather than POSTing
//! a form, because a browser-driven GET is the simplest reliable navigation. The
//! block-detection ([`crate::ddg_searcher`]) and result parsing
//! ([`crate::html_parser`]) are reused verbatim so both backends agree on what a
//! "block" and a "result" look like.

use std::sync::Arc;

use async_trait::async_trait;
use jinn_web_fetch::{FetchError, RenderedPage, SharedBrowser};

use crate::{SearchError, SearchOptions, SearchResult, WebSearcher, ddg_searcher, html_parser};

/// Fallback DDG HTML endpoint when a caller supplies an unparseable base URL.
const FALLBACK_BASE_URL: &str = "https://html.duckduckgo.com/html";

/// DuckDuckGo browser-backed searcher.
///
/// Owns an `Arc<SharedBrowser>` (shared with `web-fetch` when both select the same
/// browser mode) and the DDG base URL. Each search renders the DDG GET results page
/// through the browser, checks for an anti-bot challenge, and parses the results.
///
/// The shared browser's `render_page` is synchronous blocking work — this
/// searcher wraps it in `spawn_blocking` internally, so the blocking tab
/// operations never run on a tokio worker thread. Callers can `.await`
/// `search` directly, as the `WebSearchActor` does.
pub struct BrowserDdgSearcher {
    browser: Arc<SharedBrowser>,
    /// The DDG HTML endpoint base, without trailing slash or query.
    /// Default: `https://html.duckduckgo.com/html`.
    base_url: String,
}

impl BrowserDdgSearcher {
    /// Creates a browser-backed searcher targeting the real DuckDuckGo endpoint.
    #[must_use]
    pub fn new(browser: Arc<SharedBrowser>) -> Self {
        Self::with_base_url(browser, "https://html.duckduckgo.com/html".to_owned())
    }

    /// Creates a browser-backed searcher with a custom base URL (tests).
    #[must_use]
    pub fn with_base_url(browser: Arc<SharedBrowser>, base_url: String) -> Self {
        Self { browser, base_url }
    }

    /// Builds the DDG GET results URL for a query + options.
    ///
    /// Encodes the same fields as [`ddg_searcher::DdgSearcher::form_fields`], but as
    /// query parameters on a GET URL (a browser navigates a GET URL trivially).
    /// Extracted into a method so the URL-building test can assert against it
    /// without a browser round-trip.
    pub(crate) fn build_url(query: &str, options: &SearchOptions, base_url: &str) -> String {
        let fields = ddg_searcher::DdgSearcher::form_fields(query, options);
        // Drop the empty `b` field: an empty `b=` query param is harmless but adds
        // noise, and ddgr's GET form omits it. `b` is only required on the POST body.
        let pairs = fields.into_iter().filter(|(key, _)| *key != "b");
        let mut url = url::Url::parse(base_url).unwrap_or_else(|_| fallback_url().clone());
        {
            let mut q = url.query_pairs_mut();
            for (key, value) in pairs {
                q.append_pair(key, &value);
            }
        }
        url.to_string()
    }
}

/// Returns the parsed fallback base URL, computing it once.
///
/// The fallback is a constant, always-valid URL. The parse is infallible in
/// practice; the `expect` is localized to this one const-known-valid site.
fn fallback_url() -> &'static url::Url {
    use std::sync::OnceLock;
    static URL: OnceLock<url::Url> = OnceLock::new();
    #[expect(clippy::expect_used, reason = "FALLBACK_BASE_URL is a const valid URL")]
    URL.get_or_init(|| url::Url::parse(FALLBACK_BASE_URL).expect("valid fallback URL"))
}
#[async_trait]
impl WebSearcher for BrowserDdgSearcher {
    async fn search(
        &self,
        query: &str,
        options: &SearchOptions,
    ) -> Result<Vec<SearchResult>, SearchError> {
        self.search_observed(query, options, std::sync::Arc::new(|_| {}))
            .await
    }

    async fn search_observed(
        &self,
        query: &str,
        options: &SearchOptions,
        on_event: jinn_web_fetch::ProgressFn,
    ) -> Result<Vec<SearchResult>, SearchError> {
        if query.trim().is_empty() {
            return Err(SearchError::InvalidQuery);
        }

        let url = Self::build_url(query, options, &self.base_url);
        let page = self.render_observed(&url, on_event).await?;

        // Block detection lives in the render's tiered wait now — a
        // challenge page never reaches this point as `Ok`.
        Ok(html_parser::parse_results(&page.html, options.max_results))
    }
}

impl BrowserDdgSearcher {
    /// Renders `url` through the shared browser with progress relayed to
    /// `on_event`, mapping [`FetchError`] to [`SearchError`].
    ///
    /// `SharedBrowser::render_page_observed` is blocking; this wraps it in
    /// `spawn_blocking` so it never runs on a tokio worker thread.
    async fn render_observed(
        &self,
        url: &str,
        on_event: jinn_web_fetch::ProgressFn,
    ) -> Result<RenderedPage, SearchError> {
        let browser = Arc::clone(&self.browser);
        let url = url.to_owned();
        tokio::task::spawn_blocking(move || browser.render_page_observed(&url, &on_event))
            .await
            .map_err(|join_err| {
                tracing::error!(error = %join_err, "browser render task panicked");
                SearchError::Network
            })?
            .map_err(map_fetch_error)
    }
}

/// Maps a [`FetchError`] to a [`SearchError`].
///
/// Browser connection death and launch failures surface as `Network` from the
/// search perspective (the searcher has no browser-recovery concern — that lives in
/// `SharedBrowser`, which already retries once). Per-tab render failures and HTTP
/// status errors map to their closest search equivalent.
fn map_fetch_error(err: FetchError) -> SearchError {
    match err {
        FetchError::Timeout => SearchError::Timeout,
        FetchError::Http { status, url } => SearchError::Http { status, url },
        FetchError::InvalidUrl(url) => {
            tracing::warn!(url = %url, "browser search: invalid URL");
            SearchError::Network
        }
        FetchError::Network | FetchError::BrowserLaunch | FetchError::BrowserCrash => {
            SearchError::Network
        }
        // The render already waited (auto + human windows as configured);
        // surface the block with the headed-solve suggestion from the error
        // display.
        FetchError::Challenge { .. } => SearchError::Blocked,
        FetchError::Render(msg) => {
            tracing::warn!(error = %msg, "browser search: render failed");
            SearchError::Network
        }
        FetchError::BinaryContent { content_type } => {
            tracing::warn!(content_type = %content_type, "browser search: binary content");
            SearchError::Network
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::unwrap_in_result,
        clippy::panic_in_result_fn,
        clippy::indexing_slicing,
        clippy::map_err_ignore,
        clippy::redundant_closure,
        reason = "test assertions"
    )]
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/ddg_search_results.html");
    use jinn_web_fetch::challenge::ChallengeKind;

    fn opts() -> SearchOptions {
        SearchOptions {
            max_results: 10,
            region: "wt-wt".to_owned(),
            safe_search: true,
        }
    }

    /// A fake [`SharedBrowser`] that returns a canned rendered page, letting us
    /// exercise the searcher's block-detection and parsing without spawning
    /// Chromium. Built via the `with_factory` seam using a stub factory.
    fn fake_browser(html: &str) -> Arc<SharedBrowser> {
        use jinn_web_fetch::{HeadlessBrowser, HeadlessBrowserFactory, RenderedPage};
        use std::sync::Mutex;

        struct StubBrowser {
            html: String,
        }
        impl HeadlessBrowser for StubBrowser {
            fn render(
                &self,
                _url: &str,
                _on_progress: &dyn Fn(jinn_web_fetch::RenderProgress),
            ) -> Result<RenderedPage, FetchError> {
                Ok(RenderedPage {
                    html: self.html.clone(),
                    final_url: String::from("https://html.duckduckgo.com/html"),
                })
            }
            fn name(&self) -> &'static str {
                "stub"
            }
            fn liveness(&self) -> Result<(), FetchError> {
                Ok(())
            }
        }

        struct StubFactory {
            html: Arc<Mutex<String>>,
        }
        impl HeadlessBrowserFactory for StubFactory {
            fn launch(&self) -> Result<Arc<dyn HeadlessBrowser>, FetchError> {
                let html = self.html.lock().expect("lock").clone();
                Ok(Arc::new(StubBrowser { html }))
            }
            fn name(&self) -> &'static str {
                "stub-factory"
            }
        }

        Arc::new(SharedBrowser::with_factory(Arc::new(StubFactory {
            html: Arc::new(Mutex::new(html.to_owned())),
        })))
    }

    /// A fake [`SharedBrowser`] whose render always fails with a challenge
    /// error — the verdict the tiered wait produces for an unsolved page.
    fn challenge_browser(kind: jinn_web_fetch::challenge::ChallengeKind) -> Arc<SharedBrowser> {
        use jinn_web_fetch::{HeadlessBrowser, HeadlessBrowserFactory};

        struct ChallengeStub {
            kind: jinn_web_fetch::challenge::ChallengeKind,
        }
        impl HeadlessBrowser for ChallengeStub {
            fn render(
                &self,
                _url: &str,
                _on_progress: &dyn Fn(jinn_web_fetch::RenderProgress),
            ) -> Result<jinn_web_fetch::RenderedPage, FetchError> {
                Err(FetchError::Challenge { kind: self.kind })
            }
            fn name(&self) -> &'static str {
                "challenge-stub"
            }
            fn liveness(&self) -> Result<(), FetchError> {
                Ok(())
            }
        }

        struct ChallengeFactory(jinn_web_fetch::challenge::ChallengeKind);
        impl HeadlessBrowserFactory for ChallengeFactory {
            fn launch(&self) -> Result<Arc<dyn HeadlessBrowser>, FetchError> {
                Ok(Arc::new(ChallengeStub { kind: self.0 }))
            }
            fn name(&self) -> &'static str {
                "challenge-factory"
            }
        }

        Arc::new(SharedBrowser::with_factory(Arc::new(ChallengeFactory(
            kind,
        ))))
    }

    #[rstest::rstest]
    #[test]
    fn build_url_encodes_query_and_options_as_get_params() {
        // Given a query and options.
        let options = opts();

        // When building the URL.
        let url = BrowserDdgSearcher::build_url(
            "rust lang",
            &options,
            "https://html.duckduckgo.com/html",
        );

        // Then the query, region, and safe-search are encoded as GET params.
        let parsed = url::Url::parse(&url).expect("valid URL");
        let params: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
        assert_eq!(
            params.get("q").map(std::string::ToString::to_string),
            Some("rust lang".to_owned())
        );
        assert_eq!(
            params.get("kl").map(std::string::ToString::to_string),
            Some("wt-wt".to_owned())
        );
        assert_eq!(
            params.get("kp").map(std::string::ToString::to_string),
            Some("1".to_owned())
        );
        assert_eq!(
            params.get("k1").map(std::string::ToString::to_string),
            Some("-1".to_owned())
        );
        // The empty `b` field is omitted from GET URLs.
        assert!(!params.contains_key("b"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn search_returns_blocked_when_browser_renders_challenge_page() {
        // Given a browser-backed searcher whose browser render fails with a
        // challenge error (detection now lives in the render's tiered wait).
        let browser = challenge_browser(ChallengeKind::DdgAnomaly);
        let searcher = BrowserDdgSearcher::new(browser);

        // When searching.
        let result = searcher.search("rust", &opts()).await;

        // Then Blocked is returned (the challenge verdict surfaced as an error).
        assert!(
            matches!(result, Err(SearchError::Blocked)),
            "expected Blocked, got {result:?}"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn search_parses_results_from_browser_rendered_results_page() {
        // Given a browser-backed searcher whose browser renders the results fixture.
        let browser = fake_browser(FIXTURE);
        let searcher = BrowserDdgSearcher::new(browser);

        // When searching.
        let results = searcher
            .search("rust programming", &opts())
            .await
            .expect("ok");

        // Then results are parsed (the fixture has 5 valid results).
        assert_eq!(results.len(), 5);
        assert_eq!(results[0].title, "Rust Programming Language");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn search_returns_error_for_empty_query() {
        // Given a browser-backed searcher.
        let browser = fake_browser(FIXTURE);
        let searcher = BrowserDdgSearcher::new(browser);

        // When searching with an empty query (no render needed — fails first).
        let result = searcher.search("   ", &opts()).await;

        // Then InvalidQuery is returned.
        assert!(matches!(result, Err(SearchError::InvalidQuery)));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn search_truncates_to_max_results() {
        // Given a browser-backed searcher rendering the 5-result fixture.
        let browser = fake_browser(FIXTURE);
        let searcher = BrowserDdgSearcher::new(browser);

        // When searching with max_results = 2.
        let options = SearchOptions {
            max_results: 2,
            ..opts()
        };
        let results = searcher.search("rust", &options).await.expect("ok");

        // Then exactly 2 results are returned.
        assert_eq!(results.len(), 2);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn search_observed_relays_progress_events_to_caller() {
        // Given a searcher backed by a browser that reports one progress event.
        use jinn_web_fetch::RenderProgress;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct ProgressStub {
            seen: Arc<AtomicUsize>,
        }
        impl jinn_web_fetch::HeadlessBrowser for ProgressStub {
            fn render(
                &self,
                _url: &str,
                on_progress: &dyn Fn(RenderProgress),
            ) -> Result<jinn_web_fetch::RenderedPage, FetchError> {
                on_progress(RenderProgress::ChallengeDetected {
                    kind: ChallengeKind::DdgAnomaly,
                    url: "https://html.duckduckgo.com/html".to_owned(),
                });
                self.seen.fetch_add(1, Ordering::SeqCst);
                Ok(jinn_web_fetch::RenderedPage {
                    html: FIXTURE.to_owned(),
                    final_url: "https://html.duckduckgo.com/html".to_owned(),
                })
            }
            fn liveness(&self) -> Result<(), FetchError> {
                Ok(())
            }
            fn name(&self) -> &'static str {
                "progress-stub"
            }
        }
        struct ProgressFactory {
            seen: Arc<AtomicUsize>,
        }
        impl jinn_web_fetch::HeadlessBrowserFactory for ProgressFactory {
            fn launch(&self) -> Result<Arc<dyn jinn_web_fetch::HeadlessBrowser>, FetchError> {
                Ok(Arc::new(ProgressStub {
                    seen: Arc::clone(&self.seen),
                }))
            }
            fn name(&self) -> &'static str {
                "progress-factory"
            }
        }
        let seen = Arc::new(AtomicUsize::new(0));
        let browser = Arc::new(SharedBrowser::with_factory(Arc::new(ProgressFactory {
            seen: Arc::clone(&seen),
        })));
        let searcher = BrowserDdgSearcher::new(browser);

        let relayed = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&relayed);
        let on_event: jinn_web_fetch::ProgressFn = Arc::new(move |_p| {
            counter.fetch_add(1, Ordering::SeqCst);
        });

        // When searching with the observer attached.
        let results = searcher
            .search_observed("rust", &opts(), on_event)
            .await
            .expect("ok");

        // Then the browser fired once, the observer relayed once, and results parsed.
        assert_eq!(seen.load(Ordering::SeqCst), 1);
        assert_eq!(relayed.load(Ordering::SeqCst), 1);
        assert_eq!(results.len(), 5);
    }
}
