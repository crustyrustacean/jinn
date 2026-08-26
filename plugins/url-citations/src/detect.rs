//! Shape-based citation detection — pure functions, no I/O.
//!
//! Three rules feed the per-session citation buffer:
//!
//! - **Call rule** ([`urls_from_call_args`]): recursively walk a tool call's
//!   JSON arguments; every http(s) URL string (anywhere: nested objects,
//!   arrays) is a candidate citation, promoted only when that call's result
//!   succeeds.
//! - **Result rule** ([`citations_from_result_content`]): a successful
//!   result whose content parses as JSON contributes every object carrying
//!   both `url` (http/https) and `title` string fields — the shape Parallel's
//!   `web_search`/`web_fetch` and most search MCP servers return.
//! - **`web-search` carve-out** ([`ddg_citation`]): the builtin
//!   `web-search` tool's output is plain text and its args carry no URL, so
//!   shape detection cannot see it; rebuild the DuckDuckGo re-run URL from
//!   the `query` argument. This is the *only* tool-name-specific rule.
//!
//! Unknown shapes are ignored, never errors.

use jinn_plugin_api::PluginCitation;

/// Whether a string is an http(s) URL.
fn is_http_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

/// Call rule: every http(s) URL string found anywhere in the JSON value.
///
/// The argument JSON string is parsed (unparseable yields nothing — the
/// model's arguments are not our problem) and walked recursively; strings
/// inside arrays and nested objects all count (e.g. `{"urls": ["http://a"]}`
/// from Parallel's `web_fetch`, or `{"url": "..."}` from jinn's builtin
/// `web-fetch`).
pub fn urls_from_call_args(arguments: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments) else {
        return Vec::new();
    };
    let mut urls = Vec::new();
    collect_urls(&value, &mut urls);
    urls
}

/// Recursive walk collecting http(s) strings.
fn collect_urls(value: &serde_json::Value, urls: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) => {
            if is_http_url(s) {
                urls.push(s.clone());
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_urls(item, urls);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                collect_urls(item, urls);
            }
        }
        _ => {}
    }
}

/// Result rule: citations for every `{url, title}` object in result JSON.
///
/// The content must parse as JSON (a failed parse is not an error — plain
/// text results like the builtin `web-search` output simply yield nothing).
/// Matching objects may sit anywhere in the tree (e.g. under a `results`
/// array key). `excerpts[0]`, when present, becomes the citation content.
pub fn citations_from_result_content(content: &str) -> Vec<PluginCitation> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let mut citations = Vec::new();
    collect_url_title_objects(&value, &mut citations);
    citations
}

/// Recursive walk collecting objects with both `url` and `title` strings.
fn collect_url_title_objects(value: &serde_json::Value, out: &mut Vec<PluginCitation>) {
    match value {
        serde_json::Value::Object(map) => {
            let url = map.get("url").and_then(serde_json::Value::as_str);
            let title = map.get("title").and_then(serde_json::Value::as_str);
            if let (Some(url), Some(title)) = (url, title)
                && is_http_url(url)
            {
                out.push(PluginCitation {
                    url: url.to_owned(),
                    title: title.to_owned(),
                    content: first_excerpt(map),
                });
            }
            for item in map.values() {
                collect_url_title_objects(item, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_url_title_objects(item, out);
            }
        }
        _ => {}
    }
}

/// The first `excerpts` array string, when present (Parallel's shape).
fn first_excerpt(map: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    map.get("excerpts")
        .and_then(serde_json::Value::as_array)
        .and_then(|excerpts| excerpts.first())
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// The `web-search` carve-out: rebuild the DuckDuckGo re-run URL.
///
/// The builtin `web-search` tool's arguments carry `{"query": "..."}` and
/// its output is plain text — invisible to both generic rules. The DDG URL
/// (form-encoded query) is the "re-run this search" affordance the old core
/// collector produced; `None` means the arguments were unparseable or had
/// no query string.
pub fn ddg_citation(arguments: &str) -> Option<PluginCitation> {
    let query = serde_json::from_str::<serde_json::Value>(arguments)
        .ok()?
        .get("query")?
        .as_str()?
        .to_owned();
    Some(PluginCitation {
        url: ddg_search_url(&query),
        title: query,
        content: None,
    })
}

/// Builds `https://duckduckgo.com/?q=<form-encoded query>`.
///
/// Form-encoding ensures spaces and special characters survive terminal
/// auto-linking.
pub fn ddg_search_url(query: &str) -> String {
    let q = form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>();
    format!("https://duckduckgo.com/?q={q}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use super::*;

    // ── Call rule ────────────────────────────────────────────────────────

    #[test]
    fn call_rule_finds_top_level_url() {
        // Given builtin web-fetch arguments.
        // When extracting URLs.
        // Then the URL is found.
        assert_eq!(
            urls_from_call_args(r#"{"url":"https://example.com/page"}"#),
            vec!["https://example.com/page".to_owned()]
        );
    }

    #[test]
    fn call_rule_finds_urls_in_array() {
        // Given Parallel web_fetch arguments (urls array).
        // When extracting URLs.
        // Then both URLs are found in order.
        assert_eq!(
            urls_from_call_args(
                r#"{"urls":["https://a.example","http://b.example"],"objective":"x"}"#
            ),
            vec![
                "https://a.example".to_owned(),
                "http://b.example".to_owned()
            ]
        );
    }

    #[test]
    fn call_rule_finds_nested_url() {
        // Given arguments with a URL nested in an object.
        // When extracting URLs.
        // Then the nested URL is found.
        assert_eq!(
            urls_from_call_args(r#"{"outer":{"inner":{"url":"https://deep.example"}}}"#),
            vec!["https://deep.example".to_owned()]
        );
    }

    #[test]
    fn call_rule_ignores_non_http_schemes() {
        // Given arguments with ftp and bare-host strings.
        // When extracting URLs.
        // Then nothing is found.
        assert_eq!(
            urls_from_call_args(r#"{"a":"ftp://nope","b":"example.com","c":"mailto:x@y"}"#),
            Vec::<String>::new()
        );
    }

    #[test]
    fn call_rule_ignores_unparseable_args() {
        // Given non-JSON arguments.
        // When extracting URLs.
        // Then nothing is found and nothing panics.
        assert_eq!(urls_from_call_args("not json"), Vec::<String>::new());
    }

    // ── Result rule ──────────────────────────────────────────────────────

    #[test]
    fn result_rule_reads_parallel_web_search_shape() {
        // Given a Parallel web_search result payload.
        let content = r#"{
            "search_id": "search_e593614d82424176ae7dfce52d958cf9",
            "results": [
                {
                    "url": "https://parallel.ai/blog/series-a",
                    "title": "Parallel raises $100M Series A",
                    "publish_date": null,
                    "excerpts": ["Answer-ready excerpt text."]
                }
            ]
        }"#;

        // When extracting citations.
        let citations = citations_from_result_content(content);

        // Then the result object yields one citation with the excerpt.
        assert_eq!(citations.len(), 1);
        assert_eq!(citations[0].url, "https://parallel.ai/blog/series-a");
        assert_eq!(citations[0].title, "Parallel raises $100M Series A");
        assert_eq!(
            citations[0].content.as_deref(),
            Some("Answer-ready excerpt text.")
        );
    }

    #[test]
    fn result_rule_reads_parallel_web_fetch_shape() {
        // Given a Parallel web_fetch result payload.
        let content = r#"{
            "extract_id": "extract_4d7398dc525142b1b9a6ba4e55c64885",
            "results": [
                {
                    "url": "https://modelcontextprotocol.io/introduction",
                    "title": "What is the Model Context Protocol (MCP)?",
                    "publish_date": null,
                    "excerpts": ["markdown content"]
                }
            ]
        }"#;

        // When extracting citations.
        let citations = citations_from_result_content(content);

        // Then the result object yields one citation.
        assert_eq!(citations.len(), 1);
        assert_eq!(
            citations[0].url,
            "https://modelcontextprotocol.io/introduction"
        );
    }

    #[test]
    fn result_rule_reads_multiple_results() {
        // Given a payload with three result objects.
        let content = r#"{"results":[
            {"url":"https://a.example","title":"A"},
            {"url":"https://b.example","title":"B"},
            {"url":"https://c.example","title":"C"}
        ]}"#;

        // When extracting citations.
        // Then all three yield, in order.
        assert_eq!(citations_from_result_content(content).len(), 3);
    }

    #[test]
    fn result_rule_ignores_plain_text() {
        // Given the builtin web-search plain-text output.
        let content = "1. Title — https://example.com/thing\n   snippet text";

        // When extracting citations.
        // Then nothing is found (not JSON).
        assert!(citations_from_result_content(content).is_empty());
    }

    #[test]
    fn result_rule_ignores_json_without_url_title_pairs() {
        // Given JSON with a URL but no title pairing.
        // When extracting citations.
        // Then nothing is found.
        assert!(citations_from_result_content(r#"{"url":"https://a.example"}"#).is_empty());
        assert!(
            citations_from_result_content(r#"{"results":[{"url":"https://a.example"}]}"#)
                .is_empty()
        );
    }

    #[test]
    fn result_rule_ignores_non_http_urls() {
        // Given a result object with an ftp url.
        // When extracting citations.
        // Then nothing is found.
        assert!(
            citations_from_result_content(
                r#"{"results":[{"url":"ftp://files.example","title":"F"}]}"#
            )
            .is_empty()
        );
    }

    // ── web-search carve-out ─────────────────────────────────────────────

    #[test]
    fn ddg_citation_builds_from_query() {
        // Given builtin web-search arguments.
        // When building the carve-out citation.
        let citation = ddg_citation(r#"{"query":"rust async await"}"#);

        // Then the URL encodes the query with the title as the query.
        assert!(citation.is_some());
        let citation = citation.expect("checked");
        assert_eq!(citation.url, "https://duckduckgo.com/?q=rust+async+await");
        assert_eq!(citation.title, "rust async await");
    }

    #[test]
    fn ddg_citation_encodes_special_characters() {
        // Given a query with spaces and &.
        // When building the URL.
        // Then the query is form-encoded.
        assert_eq!(
            ddg_search_url("rust async & await"),
            "https://duckduckgo.com/?q=rust+async+%26+await"
        );
    }

    #[test]
    fn ddg_citation_encodes_empty_query() {
        // Given an empty query.
        // When building the URL.
        // Then it is the bare search endpoint.
        assert_eq!(ddg_search_url(""), "https://duckduckgo.com/?q=");
    }

    #[test]
    fn ddg_citation_none_without_query() {
        // Given arguments with no query field.
        // When building the carve-out.
        // Then None is returned.
        assert!(ddg_citation(r#"{"objective":"x"}"#).is_none());
        assert!(ddg_citation("not json").is_none());
    }
}
