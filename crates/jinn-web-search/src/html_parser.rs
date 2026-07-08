//! HTML result extraction for DuckDuckGo's `html` endpoint.
//!
//! [`parse_results`] walks the DDG results page with CSS selectors (via the
//! `scraper` crate) and returns decoded [`SearchResult`]s. The result link
//! `href`s are DDG redirect wrappers; [`decode_ddg_url`] unwraps them into the
//! final, usable URL.

use scraper::{Html, Selector};

use crate::SearchResult;

/// Selects each result container.
const RESULT_CONTAINER: &str = "div.links_main";

/// Within a container, the title anchor (carries the URL + title text).
const TITLE_LINK: &str = "h2.result__title a";

/// Within a container, the snippet anchor (carries the abstract text).
const SNIPPET: &str = "a.result__snippet";

/// Parses a DuckDuckGo HTML results page into [`SearchResult`]s.
///
/// Results whose title anchor is missing are skipped. Links whose href starts
/// with `/search` (DDG "News for" / "Images for" navigation) are skipped, as
/// ddgr does. The returned list is truncated to `max`.
///
/// Returns an empty vector for a page with no results — this is a valid
/// outcome, not an error.
#[must_use]
pub fn parse_results(html: &str, max: usize) -> Vec<SearchResult> {
    let document = Html::parse_document(html);

    let container_sel = match Selector::parse(RESULT_CONTAINER) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, selector = RESULT_CONTAINER, "invalid container selector");
            return Vec::new();
        }
    };
    let link_sel = match Selector::parse(TITLE_LINK) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, selector = TITLE_LINK, "invalid title-link selector");
            return Vec::new();
        }
    };
    let snippet_sel = match Selector::parse(SNIPPET) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, selector = SNIPPET, "invalid snippet selector");
            return Vec::new();
        }
    };

    let mut results = Vec::new();
    for container in document.select(&container_sel) {
        // The first matching title anchor is the result link.
        let Some(title_anchor) = container.select(&link_sel).next() else {
            continue;
        };

        let raw_href = title_anchor.value().attr("href").unwrap_or_default();
        // Skip DDG "News for" / "Images for" navigation links.
        if raw_href.starts_with("/search") || raw_href.is_empty() {
            continue;
        }

        let title = title_anchor.text().collect::<Vec<_>>().join("").trim().to_owned();
        if title.is_empty() {
            continue;
        }

        let url = decode_ddg_url(raw_href);
        let snippet = container
            .select(&snippet_sel)
            .next()
            .map(|a| a.text().collect::<Vec<_>>().join("").trim().to_owned())
            .unwrap_or_default();

        results.push(SearchResult {
            title,
            url,
            snippet,
        });

        if results.len() >= max {
            break;
        }
    }

    results
}

/// Decodes a DuckDuckGo redirect-wrapped `href` into the final URL.
///
/// DDG result links are not the final URL. There are two known wrapper formats:
///
/// - **Modern:** `//duckduckgo.com/l/?uddg=<encoded>&rut=...` — the final URL
///   is the percent-encoded `uddg` query parameter.
/// - **Legacy:** `?q=<encoded>&sa=...` (or `/l/?q=<encoded>&sa=...`) — the
///   final URL is the percent-encoded `q` parameter.
///
/// If neither pattern is recognized, the href is returned as-is (with `https:`
/// prepended when it is protocol-relative `//...`), matching ddgr's fallback
/// behaviour (`except ValueError: pass`).
#[must_use]
pub fn decode_ddg_url(href: &str) -> String {
    // Find the query portion (after the first '?') and scan its pairs for
    // the target URL. DDG carries it in `uddg` (modern) or `q` (legacy).
    if let Some((_, query)) = href.split_once('?') {
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            match key.as_ref() {
                "uddg" | "q" => return value.into_owned(),
                _ => {}
            }
        }
    }

    // Fallback: use the raw href. Protocol-relative URLs get an https scheme.
    if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href.to_owned()
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

    const FIXTURE: &str = include_str!("../tests/fixtures/ddg_search_results.html");

    #[test]
    fn parse_results_extracts_all_valid_results() {
        // Given the fixture (5 real results + 1 `/search` nav link + nav-link).
        // When parsing.
        let results = parse_results(FIXTURE, 50);

        // Then 5 results are extracted (the `/search` link and nav-link are skipped).
        assert_eq!(results.len(), 5, "5 valid results; nav/search links skipped");
    }

    #[test]
    fn parse_results_extracts_title_and_url_of_first_result() {
        // Given the fixture.
        // When parsing.
        let results = parse_results(FIXTURE, 50);

        // Then the first result is the rust-lang.org entry with a decoded URL.
        let first = &results[0];
        assert_eq!(first.title, "Rust Programming Language");
        assert_eq!(first.url, "https://www.rust-lang.org/");
    }

    #[test]
    fn parse_results_extracts_snippet_of_first_result() {
        // Given the fixture.
        // When parsing.
        let results = parse_results(FIXTURE, 50);

        // Then the first result's snippet is present (non-empty).
        assert!(!results[0].snippet.is_empty());
        assert!(results[0].snippet.contains("reliable and efficient software"));
    }

    #[test]
    fn parse_results_decodes_legacy_q_redirect_format() {
        // Given the fixture (result 3 uses ?q=...&sa=... ).
        // When parsing.
        let results = parse_results(FIXTURE, 50);

        // Then the GitHub result URL is decoded from the legacy format.
        let github = results
            .iter()
            .find(|r| r.title.contains("GitHub"))
            .expect("github result present");
        assert_eq!(github.url, "https://github.com/rust-lang/rust");
    }

    #[test]
    fn parse_results_keeps_plain_url_without_wrapper() {
        // Given the fixture (result 5 is a plain https url, no wrapper).
        // When parsing.
        let results = parse_results(FIXTURE, 50);

        // Then the forum URL is kept verbatim.
        let forum = results
            .iter()
            .find(|r| r.title.contains("Forum"))
            .expect("forum result present");
        assert_eq!(forum.url, "https://users.rust-lang.org/");
    }

    #[test]
    fn parse_results_snippet_empty_when_absent() {
        // Given the fixture (result 4 "Rust Jobs" has no snippet).
        // When parsing.
        let results = parse_results(FIXTURE, 50);

        // Then its snippet is the empty string.
        let jobs = results
            .iter()
            .find(|r| r.title.contains("Rust Jobs"))
            .expect("jobs result present");
        assert!(jobs.snippet.is_empty());
    }

    #[test]
    fn parse_results_truncates_to_max() {
        // Given the fixture (5 valid results).
        // When parsing with max = 2.
        let results = parse_results(FIXTURE, 2);

        // Then only 2 results are returned.
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn parse_results_returns_empty_for_page_with_no_results() {
        // Given an HTML page with no result containers.
        let html = "<html><body><p>nothing here</p></body></html>";

        // When parsing.
        let results = parse_results(html, 10);

        // Then an empty vec is returned (not an error).
        assert!(results.is_empty());
    }

    #[test]
    fn decode_ddg_url_modern_uddg_format() {
        // Given a modern redirect href.
        let href = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpath&rut=x";

        // When decoding.
        let url = decode_ddg_url(href);

        // Then the final URL is extracted from the uddg param.
        assert_eq!(url, "https://example.com/path");
    }

    #[test]
    fn decode_ddg_url_legacy_q_format() {
        // Given a legacy redirect href.
        let href = "/l/?q=https%3A%2F%2Fexample.com%2F&sa=U";

        // When decoding.
        let url = decode_ddg_url(href);

        // Then the final URL is extracted from the q param.
        assert_eq!(url, "https://example.com/");
    }

    #[test]
    fn decode_ddg_url_protocol_relative_fallback() {
        // Given a protocol-relative href with no recognized query param.
        let href = "//some.example.com/path";

        // When decoding.
        let url = decode_ddg_url(href);

        // Then https is prepended.
        assert_eq!(url, "https://some.example.com/path");
    }

    #[test]
    fn decode_ddg_url_plain_url_passthrough() {
        // Given a plain absolute URL with no query string.
        let href = "https://users.rust-lang.org/";

        // When decoding.
        let url = decode_ddg_url(href);

        // Then it is returned unchanged.
        assert_eq!(url, "https://users.rust-lang.org/");
    }
}
