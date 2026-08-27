//! The first-party URL-citations plugin.
//!
//! Subscribes to `tool_call` / `tool_result` / `turn_end` host events and
//! detects citable web sources by shape (see [`detect`]): URLs appearing in
//! tool-call arguments, and `{url/link, title}` objects in successful
//! tool-result JSON — both descended into strings that embed further JSON,
//! bounded — plus one builtin `web-search` carve-out that rebuilds the
//! DuckDuckGo re-run URL from the query. Detected citations accumulate in a
//! per-session buffer, deduplicated by URL, and flush as one
//! `PushCitations` contribution when a turn reaches a final answer.
//!
//! Wire behavior: `Hello` (with subscriptions) → (await `Welcome`) → event
//! loop until stdin closes.

mod detect;

use std::collections::HashMap;
use std::io::BufRead as _;

use jinn_plugin_api::{
    HostToPlugin, PluginCitation, PluginToHost, PluginToHostOrHostToPlugin, PushCitations,
    ToolCallEvent, ToolResultEvent, TurnEndEvent,
};
use jinn_plugin_sdk::{PluginOutput, hello_with_subscriptions, push, welcome};

/// Turn-scoped detection state: pending call-rule candidates and the
/// per-session citation buffer.
struct CitationState {
    /// Call-rule candidates keyed by `tool_call_id`, awaiting their result.
    pending: HashMap<String, Vec<PluginCitation>>,
    /// Confirmed citations per session, deduplicated by URL, in order.
    buffer: HashMap<String, Vec<PluginCitation>>,
}

impl CitationState {
    fn new() -> Self {
        Self {
            pending: HashMap::new(),
            buffer: HashMap::new(),
        }
    }

    /// Stashes call-rule candidates (and the `web-search` carve-out) for a
    /// tool call, keyed by its id.
    fn on_tool_call(&mut self, event: &ToolCallEvent) {
        let mut candidates = Vec::new();
        if event.name == "web-search"
            && let Some(citation) = detect::ddg_citation(&event.arguments)
        {
            candidates.push(citation);
        }
        for url in detect::urls_from_call_args(&event.arguments) {
            candidates.push(PluginCitation {
                url,
                title: String::new(),
                content: None,
            });
        }
        if !candidates.is_empty() {
            self.pending.insert(event.tool_call_id.clone(), candidates);
        }
    }

    /// Promotes pending candidates on success, extracts result-rule
    /// citations, and stashes both into the session buffer (deduped).
    fn on_tool_result(&mut self, event: &ToolResultEvent) {
        if !event.success {
            // A failed call is not citable — drop its candidates.
            self.pending.remove(&event.tool_call_id);
            return;
        }
        let mut citations = self.pending.remove(&event.tool_call_id).unwrap_or_default();
        for citation in detect::citations_from_result_content(&event.content) {
            citations.push(citation);
        }
        self.record(&event.session_id, citations);
    }

    /// Flushes and returns the session's buffered citations on a final
    /// answer; retains them otherwise (a later successful turn still
    /// surfaces the sources).
    fn on_turn_end(&mut self, event: &TurnEndEvent) -> Option<Vec<PluginCitation>> {
        if !event.final_answer {
            return None;
        }
        let citations = self.buffer.remove(&event.session_id)?;
        (!citations.is_empty()).then_some(citations)
    }

    /// Appends citations to the session buffer, deduplicating by URL.
    fn record(&mut self, session_id: &str, citations: Vec<PluginCitation>) {
        let buffered = self.buffer.entry(session_id.to_owned()).or_default();
        for citation in citations {
            // The call rule stashes URLs with an empty title; a later
            // same-URL citation with a real title wins.
            if let Some(existing) = buffered.iter_mut().find(|c| c.url == citation.url) {
                if existing.title.is_empty() {
                    *existing = citation;
                }
                continue;
            }
            buffered.push(citation);
        }
    }
}

fn main() {
    let mut out = PluginOutput::stdout();
    if hello_with_subscriptions(
        &mut out,
        "url-citations",
        &["tool_call", "tool_result", "turn_end"],
    )
    .is_err()
    {
        return;
    }
    if welcome().is_err() {
        return;
    }

    // Ordering assumption (documented per the task spec): the host forwards
    // `tool_call`/`tool_result` synchronously along the tool loop, and this
    // plugin processes them and pushes `PushCitations` before the next LLM
    // dispatch begins — so a flush at `turn_end` always lands within the
    // correct turn. If citations ever appear in the WRONG turn's footer,
    // this assumption has been violated (e.g. the forwarder or this guest
    // added buffering/async hops); re-synchronize before adding retries.
    let mut state = CitationState::new();
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    while let Some(Ok(line)) = lines.next() {
        let Ok(envelope) = serde_json::from_str::<jinn_plugin_api::Envelope>(&line) else {
            continue;
        };
        let event = match envelope.msg {
            PluginToHostOrHostToPlugin::Host(event) => event,
            _ => continue,
        };
        match event {
            HostToPlugin::ToolCallEvent(e) => state.on_tool_call(&e),
            HostToPlugin::ToolResultEvent(e) => state.on_tool_result(&e),
            HostToPlugin::TurnEndEvent(e) => {
                if let Some(citations) = state.on_turn_end(&e) {
                    let _ = push(
                        &mut out,
                        PluginToHost::PushCitations(PushCitations {
                            session_id: e.session_id.clone(),
                            citations,
                        }),
                    );
                }
            }
            HostToPlugin::Welcome(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use super::*;

    /// A tool-call event factory.
    fn call(id: &str, name: &str, arguments: &str) -> ToolCallEvent {
        ToolCallEvent {
            session_id: "s-1".to_owned(),
            tool_call_id: id.to_owned(),
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        }
    }

    /// A tool-result event factory.
    fn result(id: &str, name: &str, content: &str, success: bool) -> ToolResultEvent {
        ToolResultEvent {
            session_id: "s-1".to_owned(),
            tool_call_id: id.to_owned(),
            name: name.to_owned(),
            content: content.to_owned(),
            success,
        }
    }

    /// A turn-end event factory.
    fn turn_end(session_id: &str, final_answer: bool) -> TurnEndEvent {
        TurnEndEvent {
            session_id: session_id.to_owned(),
            final_answer,
        }
    }

    #[test]
    fn call_rule_candidates_promote_on_success() {
        // Given a builtin web-fetch call and its successful result.
        let mut state = CitationState::new();
        state.on_tool_call(&call("c1", "web-fetch", r#"{"url":"https://example.com"}"#));

        // When the result succeeds (plain-text page content).
        state.on_tool_result(&result("c1", "web-fetch", "# page markdown", true));

        // Then the URL is buffered for the session.
        state
            .on_turn_end(&turn_end("s-1", true))
            .expect("flush on final answer")
            .iter()
            .find(|c| c.url == "https://example.com")
            .expect("call-rule citation promoted");
    }

    #[test]
    fn call_rule_candidates_discarded_on_failure() {
        // Given a builtin web-fetch call and its failed result.
        let mut state = CitationState::new();
        state.on_tool_call(&call("c1", "web-fetch", r#"{"url":"https://example.com"}"#));

        // When the result fails.
        state.on_tool_result(&result("c1", "web-fetch", "fetch failed", false));

        // Then nothing is buffered — a failed fetch is not citable.
        assert!(state.on_turn_end(&turn_end("s-1", true)).is_none());
    }

    #[test]
    fn parallel_search_result_yields_citations() {
        // Given a parallel web_search call and its JSON result.
        let mut state = CitationState::new();
        state.on_tool_call(&call(
            "c1",
            "mcp__parallel__web_search",
            r#"{"objective":"find docs","search_queries":["rust docs"]}"#,
        ));
        let content = r#"{"search_id":"x","results":[{"url":"https://doc.rust-lang.org","title":"The Rust Book","publish_date":null,"excerpts":["Learn Rust."]}]}"#;

        // When the result succeeds.
        state.on_tool_result(&result("c1", "mcp__parallel__web_search", content, true));

        // Then the flushed citations carry the result-rule entry with its
        // excerpt as content.
        let flushed = state.on_turn_end(&turn_end("s-1", true)).expect("flush");
        let citation = flushed
            .iter()
            .find(|c| c.url == "https://doc.rust-lang.org")
            .expect("result-rule citation present");
        assert_eq!(citation.title, "The Rust Book");
        assert_eq!(citation.content.as_deref(), Some("Learn Rust."));
    }

    #[test]
    fn web_search_carve_out_flushes_ddg_url() {
        // Given a builtin web-search call and its plain-text result.
        let mut state = CitationState::new();
        state.on_tool_call(&call("c1", "web-search", r#"{"query":"rust async"}"#));

        // When the result succeeds (text the shape rules can't see).
        state.on_tool_result(&result(
            "c1",
            "web-search",
            "1. Title — url\n snippet",
            true,
        ));

        // Then the flush carries the DDG re-run URL with the encoded query.
        let flushed = state.on_turn_end(&turn_end("s-1", true)).expect("flush");
        assert!(
            flushed
                .iter()
                .any(|c| c.url == "https://duckduckgo.com/?q=rust+async")
        );
    }

    #[test]
    fn zai_turn_buffers_and_flushes_deduped_citations() {
        // Given a Z.ai web_search_prime call whose result content is a
        // doubly-encoded array of {title, link, content, refer} entries —
        // one entry sharing another's URL to exercise dedup across rules.
        let mut state = CitationState::new();
        state.on_tool_call(&call(
            "c1",
            "mcp__zai-web-search-prime__web_search_prime",
            r#"{"search_query":"mega man legends series","location":"us"}"#,
        ));
        let wrapped = |entries: &str| {
            let escaped = entries.replace('"', "\\\"");
            format!("\"[{escaped}]\"")
        };
        let content = wrapped(
            r#"{"title":"Mega Man Legends (series) - MMKB - Fandom","link":"https://megaman.fandom.com/wiki/Mega_Man_Legends_(series)","content":"It is centered around MegaMan Volnutt.","refer":"ref_1"},{"title":"Mega Man Legends","link":"https://en.wikipedia.org/wiki/Mega_Man_Legends","content":"The player controls Mega Man Volnutt.","refer":"ref_2"},{"title":"Same Page Again","link":"https://megaman.fandom.com/wiki/Mega_Man_Legends_(series)","content":"duplicate url","refer":"ref_3"}"#,
        );

        // When the result succeeds and the turn reaches a final answer.
        state.on_tool_result(&result(
            "c1",
            "mcp__zai-web-search-prime__web_search_prime",
            &content,
            true,
        ));
        let flushed = state
            .on_turn_end(&turn_end("s-1", true))
            .expect("zai turn flushes");

        // Then the flushed citations carry each unique source in payload
        // order with title + snippet, the duplicate URL appearing once.
        assert_eq!(flushed.len(), 2);
        assert_eq!(
            flushed[0].url,
            "https://megaman.fandom.com/wiki/Mega_Man_Legends_(series)"
        );
        assert_eq!(
            flushed[0].title,
            "Mega Man Legends (series) - MMKB - Fandom"
        );
        assert_eq!(
            flushed[0].content.as_deref(),
            Some("It is centered around MegaMan Volnutt.")
        );
        assert_eq!(
            flushed[1].url,
            "https://en.wikipedia.org/wiki/Mega_Man_Legends"
        );
    }

    #[test]
    fn buffer_dedups_by_url_across_rules() {
        // Given a parallel web_search citing a URL, then a web_fetch of it.
        let mut state = CitationState::new();
        let content =
            r#"{"results":[{"url":"https://same.example","title":"Titled","excerpts":["e"]}]}"#;
        state.on_tool_call(&call("c1", "mcp__parallel__web_search", "{}"));
        state.on_tool_result(&result("c1", "mcp__parallel__web_search", content, true));
        state.on_tool_call(&call(
            "c2",
            "mcp__parallel__web_fetch",
            r#"{"urls":["https://same.example"]}"#,
        ));
        state.on_tool_result(&result("c2", "mcp__parallel__web_fetch", "{}", true));

        // When the turn ends.
        let flushed = state.on_turn_end(&turn_end("s-1", true)).expect("flush");

        // Then the URL appears once, keeping the titled entry.
        let matches: Vec<_> = flushed
            .iter()
            .filter(|c| c.url == "https://same.example")
            .collect();
        assert_eq!(matches.len(), 1, "deduped by URL");
        assert_eq!(matches[0].title, "Titled", "titled entry wins");
    }

    #[test]
    fn errored_turn_retains_buffer_for_next_turn() {
        // Given a buffered citation and a non-final turn end.
        let mut state = CitationState::new();
        state.on_tool_call(&call("c1", "web-fetch", r#"{"url":"https://example.com"}"#));
        state.on_tool_result(&result("c1", "web-fetch", "ok", true));

        // When the turn ends without a final answer.
        assert!(state.on_turn_end(&turn_end("s-1", false)).is_none());

        // Then the next successful turn still flushes the citation.
        assert!(
            state
                .on_turn_end(&turn_end("s-1", true))
                .expect("retained citation flushes next turn")
                .iter()
                .any(|c| c.url == "https://example.com")
        );
    }

    #[test]
    fn flush_clears_the_session_buffer() {
        // Given a flushed citation.
        let mut state = CitationState::new();
        state.on_tool_call(&call("c1", "web-fetch", r#"{"url":"https://example.com"}"#));
        state.on_tool_result(&result("c1", "web-fetch", "ok", true));
        let _ = state.on_turn_end(&turn_end("s-1", true)).expect("flush");

        // When the next turn ends successfully with no new citations.
        // Then nothing flushes (the buffer was cleared).
        assert!(state.on_turn_end(&turn_end("s-1", true)).is_none());
    }

    #[test]
    fn sessions_are_isolated() {
        // Given citations buffered for two sessions.
        let mut state = CitationState::new();
        state.on_tool_call(&ToolCallEvent {
            session_id: "s-a".to_owned(),
            tool_call_id: "c1".to_owned(),
            name: "web-fetch".to_owned(),
            arguments: r#"{"url":"https://a.example"}"#.to_owned(),
        });
        state.on_tool_call(&ToolCallEvent {
            session_id: "s-b".to_owned(),
            tool_call_id: "c2".to_owned(),
            name: "web-fetch".to_owned(),
            arguments: r#"{"url":"https://b.example"}"#.to_owned(),
        });
        state.on_tool_result(&ToolResultEvent {
            session_id: "s-a".to_owned(),
            tool_call_id: "c1".to_owned(),
            name: "web-fetch".to_owned(),
            content: "ok".to_owned(),
            success: true,
        });
        state.on_tool_result(&ToolResultEvent {
            session_id: "s-b".to_owned(),
            tool_call_id: "c2".to_owned(),
            name: "web-fetch".to_owned(),
            content: "ok".to_owned(),
            success: true,
        });

        // When session A's turn ends.
        let flushed = state.on_turn_end(&turn_end("s-a", true)).expect("flush A");

        // Then only A's citation flushed; B's is retained.
        assert_eq!(flushed.len(), 1);
        assert_eq!(flushed[0].url, "https://a.example");
        let flushed_b = state.on_turn_end(&turn_end("s-b", true)).expect("flush B");
        assert_eq!(flushed_b.len(), 1);
        assert_eq!(flushed_b[0].url, "https://b.example");
    }

    #[test]
    fn call_rule_url_title_falls_back_via_merge() {
        // Given a call-rule URL (empty title) later matched by a titled
        // result-rule citation in the same turn.
        let mut state = CitationState::new();
        state.on_tool_call(&call(
            "c1",
            "mcp__parallel__web_fetch",
            r#"{"urls":["https://x.example"]}"#,
        ));
        let content = r#"{"results":[{"url":"https://x.example","title":"X Title"}]}"#;
        state.on_tool_result(&result("c1", "mcp__parallel__web_fetch", content, true));

        // When the turn ends.
        let flushed = state.on_turn_end(&turn_end("s-1", true)).expect("flush");

        // Then the single citation carries the title (merge on dedup).
        let matches: Vec<_> = flushed
            .iter()
            .filter(|c| c.url == "https://x.example")
            .collect();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].title, "X Title");
    }

    #[test]
    fn empty_buffer_flush_pushes_nothing() {
        // Given no buffered citations.
        let mut state = CitationState::new();

        // When a final-answer turn ends.
        // Then no flush payload is produced.
        assert!(state.on_turn_end(&turn_end("s-1", true)).is_none());
    }
}
