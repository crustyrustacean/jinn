//! The citation collector actor.
//!
//! See the [module docs](super) for the high-level flow. This module owns the
//! actor struct, its startup subscriptions, and the per-message handlers that
//! accumulate consulted sources and flush them at turn end.
//!
//! # Shutdown
//!
//! In-memory state only — nothing to release during [`Actor::on_stop`].

use std::collections::HashMap;

use kameo::actor::ActorRef;
use kameo::prelude::{Actor, Context, Message};

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::state::State;
use crate::feat::citation_collector::buffer::TurnCitationBuffer;
use crate::feat::session::chat_entry::ChatEntryKind;
use crate::feat::session::phase_machine::PhaseKind;
use crate::feat::session::protocol::citations_received::CitationsReceived;
use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
use crate::feat::tools_actor::protocol::command::{ExecuteWebFetch, ExecuteWebSearch};
use crate::feat::tools_actor::protocol::event::ToolExecutionCompleted;
use crate::protocol::SessionId;

use jinn_provider::tool_types::ToolCall;

/// A web source stashed at tool-dispatch time, awaiting its execution result.
///
/// `ExecuteWebFetch` / `ExecuteWebSearch` carry the URL / query but fire
/// *before* execution completes. We stash these keyed by `tool_call_id`, then
/// promote only the successful ones into the citation buffer when
/// [`ToolExecutionCompleted`] arrives. Failed fetches/searches are dropped (a
/// failed page is not a citable source).
#[derive(Debug, Clone)]
struct PendingSource {
    /// Which session's turn this source belongs to.
    session_id: SessionId,
    /// The URL to cite (the fetched page URL, or the DDG search URL).
    url: String,
    /// Human-readable label for the citation (the URL itself for fetches, the
    /// query text for searches).
    title: String,
}

/// Arguments parsed from a `web-fetch` tool call's JSON arguments string.
#[derive(serde::Deserialize)]
struct WebFetchArgs {
    url: String,
}

/// Arguments parsed from a `web-search` tool call's JSON arguments string.
#[derive(serde::Deserialize)]
struct WebSearchArgs {
    query: String,
}

/// The citation collector actor.
///
/// Accumulates `web-fetch` / `web-search` sources across a turn and flushes them
/// as a single [`CitationsReceived`] event when the turn reaches a genuine
/// assistant answer. See the [module docs](super) for the full flow.
pub struct CitationCollectorActor {
    deps: ActorDeps,
    state: State,
    buffer: TurnCitationBuffer,
    pending_sources: HashMap<String, PendingSource>,
}

/// Dependencies for [`CitationCollectorActor`].
#[derive(Clone)]
pub struct CitationCollectorActorDeps {
    /// Universal actor dependencies (services + bus).
    pub deps: ActorDeps,
    /// Shared application state (read-only access to session history).
    pub state: State,
}

impl Actor for CitationCollectorActor {
    type Args = CitationCollectorActorDeps;
    type Error = std::convert::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        // Subscribe to tool-dispatch commands (carry the URL/query, pre-execution)
        // and the completion event (carries success/failure). Correlate by
        // `tool_call_id`.
        args.deps
            .subscribe(actor_ref.clone().recipient::<ExecuteWebFetch>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<ExecuteWebSearch>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<ToolExecutionCompleted>())
            .await;
        // The flush trigger: every Streaming → Idle transition.
        args.deps
            .subscribe(actor_ref.recipient::<SessionPhaseChanged>())
            .await;

        Ok(Self {
            deps: args.deps,
            state: args.state,
            buffer: TurnCitationBuffer::new(),
            pending_sources: HashMap::new(),
        })
    }
}

impl BusPublish for CitationCollectorActor {
    fn bus(&self) -> &crate::common::services::bus_service::BusService {
        self.deps.bus()
    }
}

impl Message<ExecuteWebFetch> for CitationCollectorActor {
    type Reply = ();

    async fn handle(&mut self, msg: ExecuteWebFetch, _ctx: &mut Context<Self, Self::Reply>) {
        self.stash_fetch(&msg.session_id, &msg.tool_call);
    }
}

impl Message<ExecuteWebSearch> for CitationCollectorActor {
    type Reply = ();

    async fn handle(&mut self, msg: ExecuteWebSearch, _ctx: &mut Context<Self, Self::Reply>) {
        self.stash_search(&msg.session_id, &msg.tool_call);
    }
}

impl Message<ToolExecutionCompleted> for CitationCollectorActor {
    type Reply = ();

    async fn handle(&mut self, msg: ToolExecutionCompleted, _ctx: &mut Context<Self, Self::Reply>) {
        self.resolve_pending(&msg);
    }
}

impl Message<SessionPhaseChanged> for CitationCollectorActor {
    type Reply = ();

    async fn handle(&mut self, msg: SessionPhaseChanged, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_phase_changed(&msg).await;
    }
}

impl CitationCollectorActor {
    /// Stash a `web-fetch` source by its `tool_call_id`.
    ///
    /// Title = the URL itself (the fetched page's address is the clearest
    /// label we have without fetching it).
    fn stash_fetch(&mut self, session_id: &SessionId, tool_call: &ToolCall) {
        let Ok(args) = serde_json::from_str::<WebFetchArgs>(&tool_call.arguments) else {
            tracing::warn!(
                tool_call_id = %tool_call.id,
                "citation-collector: could not parse web-fetch arguments; skipping"
            );
            return;
        };
        self.pending_sources.insert(
            tool_call.id.clone(),
            PendingSource {
                session_id: session_id.clone(),
                url: args.url.clone(),
                title: args.url,
            },
        );
    }

    /// Stash a `web-search` source by its `tool_call_id`.
    ///
    /// The cited URL is the DuckDuckGo HTML search URL — a "re-run this search"
    /// affordance. Title = the query text.
    fn stash_search(&mut self, session_id: &SessionId, tool_call: &ToolCall) {
        let Ok(args) = serde_json::from_str::<WebSearchArgs>(&tool_call.arguments) else {
            tracing::warn!(
                tool_call_id = %tool_call.id,
                "citation-collector: could not parse web-search arguments; skipping"
            );
            return;
        };
        let search_url = build_ddg_search_url(&args.query);
        self.pending_sources.insert(
            tool_call.id.clone(),
            PendingSource {
                session_id: session_id.clone(),
                url: search_url,
                title: args.query,
            },
        );
    }

    /// On tool completion, promote a stashed source into the buffer on success,
    /// or discard it on failure. Only `web-fetch` / `web-search` are tracked.
    fn resolve_pending(&mut self, msg: &ToolExecutionCompleted) {
        if !matches!(msg.result.name.as_str(), "web-fetch" | "web-search") {
            return;
        }
        let Some(src) = self.pending_sources.remove(&msg.result.tool_call_id) else {
            return;
        };
        if msg.result.success {
            self.buffer.record(
                &src.session_id,
                jinn_provider::UrlCitation {
                    url: src.url,
                    title: src.title,
                    content: None,
                    start_index: None,
                    end_index: None,
                },
            );
        }
        // On failure, the source is already removed and simply discarded —
        // a failed fetch/search is not a citable source.
    }

    /// On `Streaming → Idle`, flush the buffer only if the session's last
    /// history entry is an assistant message — i.e. the turn produced a real
    /// final answer. Error / cancel mid-turn leaves a non-assistant last entry,
    /// so we retain the buffer for a later successful turn.
    async fn handle_phase_changed(&mut self, msg: &SessionPhaseChanged) {
        if !(msg.old_phase == PhaseKind::Streaming && msg.new_phase == PhaseKind::Idle) {
            return;
        }

        let is_assistant = self
            .state
            .read()
            .try_session(&msg.session_id)
            .and_then(|session| session.history().last())
            .is_some_and(|entry| matches!(entry.kind, ChatEntryKind::Assistant(_)));

        if !is_assistant {
            return;
        }

        if let Some(citations) = self.buffer.flush(&msg.session_id) {
            tracing::info!(
                session_id = %msg.session_id,
                count = citations.len(),
                "citation-collector: flushing citations"
            );
            let () = self
                .publish(CitationsReceived {
                    session_id: msg.session_id.clone(),
                    citations,
                })
                .await;
        }
    }
}

/// Build a DuckDuckGo HTML search URL for a query: `https://duckduckgo.com/?q=...`.
///
/// This is the "re-run this search" link surfaced in the footer. Form-encoding
/// the query ensures spaces and special characters survive terminal
/// auto-linking.
fn build_ddg_search_url(query: &str) -> String {
    let q = form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>();
    format!("https://duckduckgo.com/?q={q}")
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unreachable,
        reason = "test code"
    )]

    use super::*;

    #[test]
    fn build_ddg_search_url_encodes_spaces_and_special_chars() {
        // Given a query with spaces and a special char.
        // When building the search URL.
        let url = build_ddg_search_url("rust async & await");

        // Then spaces and '&' are form-encoded.
        assert!(url.starts_with("https://duckduckgo.com/?q="));
        assert!(url.contains("rust+async"));
        assert!(url.contains("%26")); // '&'
        assert!(url.contains("await"));
    }

    #[test]
    fn build_ddg_search_url_encodes_empty_query() {
        // Given an empty query.
        // When building the search URL.
        let url = build_ddg_search_url("");

        // Then it is the bare search endpoint with an empty q parameter.
        assert_eq!(url, "https://duckduckgo.com/?q=");
    }
}

#[cfg(test)]
mod actor_tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "test assertions"
    )]

    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::services::{BusAudit, BusService, Services};
    use crate::common::state::State;
    use crate::feat::session::chat_entry::ChatEntry;
    use crate::feat::tools_actor::tool_types::ToolResult;

    /// Build a collector actor backed by a recording bus + default state.
    async fn create_actor() -> (CitationCollectorActor, BusAudit) {
        let (bus, audit) = BusService::new_recording();
        let services = Services::new_fake_with_bus(bus).await;
        let actor = CitationCollectorActor {
            deps: ActorDeps { services },
            state: State::new(AppState::default()),
            buffer: TurnCitationBuffer::new(),
            pending_sources: HashMap::new(),
        };
        (actor, audit)
    }

    /// Seed a session's history with one entry.
    fn seed_history(actor: &CitationCollectorActor, sid: &SessionId, entry: ChatEntry) {
        let mut state = actor.state.write();
        let session = state.session_mut_or_create(sid);
        session.push_entry(entry);
    }

    fn sid() -> SessionId {
        SessionId::new()
    }

    fn fetch_call(id: &str, url: &str) -> ExecuteWebFetch {
        ExecuteWebFetch {
            session_id: sid(),
            tool_call: ToolCall {
                id: id.to_owned(),
                name: "web-fetch".to_owned(),
                arguments: format!("{{\"url\":\"{url}\"}}"),
            },
        }
    }

    fn search_call(id: &str, query: &str) -> ExecuteWebSearch {
        ExecuteWebSearch {
            session_id: sid(),
            tool_call: ToolCall {
                id: id.to_owned(),
                name: "web-search".to_owned(),
                arguments: format!("{{\"query\":\"{query}\"}}"),
            },
        }
    }

    fn completed(id: &str, name: &str, success: bool) -> ToolExecutionCompleted {
        ToolExecutionCompleted {
            session_id: sid(), // overwritten by callers where needed
            result: ToolResult {
                tool_call_id: id.to_owned(),
                name: name.to_owned(),
                content: String::new(),
                success,
                full_content: None,
                truncation: None,
                pin_position: None,
            },
        }
    }

    #[tokio::test]
    async fn web_fetch_success_promotes_url_into_buffer() {
        // Given a collector.
        let (mut actor, _audit) = create_actor().await;
        let s = sid();
        let mut cmd = fetch_call("tc1", "https://example.com/");
        cmd.session_id = s.clone();

        // When stashing the fetch then completing it successfully.
        actor.stash_fetch(&cmd.session_id, &cmd.tool_call);
        let mut done = completed("tc1", "web-fetch", true);
        done.session_id = s.clone();
        actor.resolve_pending(&done);

        // Then the buffer holds the fetched URL as a citation.
        assert!(actor.buffer.pending(&s));
    }

    #[tokio::test]
    async fn web_fetch_failure_drops_source_and_does_not_cite() {
        // Given a collector with a stashed fetch.
        let (mut actor, _audit) = create_actor().await;
        let s = sid();
        let mut cmd = fetch_call("tc1", "https://example.com/");
        cmd.session_id = s.clone();
        actor.stash_fetch(&cmd.session_id, &cmd.tool_call);

        // When the fetch completes with failure.
        let mut done = completed("tc1", "web-fetch", false);
        done.session_id = s.clone();
        actor.resolve_pending(&done);

        // Then nothing is buffered and the pending source is gone.
        assert!(!actor.buffer.pending(&s));
        assert!(actor.pending_sources.is_empty());
    }

    #[tokio::test]
    async fn web_search_success_cites_ddg_search_url_not_result_urls() {
        // Given a collector.
        let (mut actor, _audit) = create_actor().await;
        let s = sid();
        let mut cmd = search_call("tc1", "rust async");
        cmd.session_id = s.clone();

        // When stashing the search then completing it successfully.
        actor.stash_search(&cmd.session_id, &cmd.tool_call);
        let mut done = completed("tc1", "web-search", true);
        done.session_id = s.clone();
        actor.resolve_pending(&done);

        // Then the buffer holds the DDG search URL (not a result URL).
        let citations = actor.buffer.flush(&s).expect("should have a citation");
        assert_eq!(citations.len(), 1);
        assert!(citations[0].url.starts_with("https://duckduckgo.com/?q="));
        assert_eq!(citations[0].title, "rust async");
    }

    #[tokio::test]
    async fn flush_publishes_citations_when_last_entry_is_assistant() {
        // Given a collector with one successful fetch buffered, and history
        // ending in an Assistant entry.
        let (mut actor, audit) = create_actor().await;
        let s = sid();
        let mut cmd = fetch_call("tc1", "https://example.com/");
        cmd.session_id = s.clone();
        actor.stash_fetch(&cmd.session_id, &cmd.tool_call);
        actor.resolve_pending(&completed("tc1", "web-fetch", true));
        seed_history(&actor, &s, ChatEntry::assistant("the answer"));

        // When the phase transitions Streaming → Idle.
        actor
            .handle_phase_changed(&SessionPhaseChanged {
                session_id: s.clone(),
                old_phase: PhaseKind::Streaming,
                new_phase: PhaseKind::Idle,
            })
            .await;

        // Then exactly one CitationsReceived was published with the URL.
        let cites: Vec<CitationsReceived> = audit.of_type::<CitationsReceived>();
        assert_eq!(cites.len(), 1);
        assert_eq!(cites[0].citations.len(), 1);
        assert_eq!(cites[0].citations[0].url, "https://example.com/");
        // And the buffer is now empty (re-flush is a no-op).
        assert!(!actor.buffer.pending(&s));
    }

    #[tokio::test]
    async fn retain_buffer_when_last_entry_is_error_then_flush_later() {
        // Given a collector with one fetch buffered, and history ending in
        // an Error entry (mid-turn failure).
        let (mut actor, audit) = create_actor().await;
        let s = sid();
        let mut cmd = fetch_call("tc1", "https://example.com/");
        cmd.session_id = s.clone();
        actor.stash_fetch(&cmd.session_id, &cmd.tool_call);
        actor.resolve_pending(&completed("tc1", "web-fetch", true));
        seed_history(&actor, &s, ChatEntry::error("stream failed"));

        // When the phase transitions Streaming → Idle (error mid-turn).
        actor
            .handle_phase_changed(&SessionPhaseChanged {
                session_id: s.clone(),
                old_phase: PhaseKind::Streaming,
                new_phase: PhaseKind::Idle,
            })
            .await;

        // Then no citation was published...
        let cites: Vec<CitationsReceived> = audit.of_type::<CitationsReceived>();
        assert!(cites.is_empty(), "no flush on error last entry");
        // ...but the buffer is retained for the next turn.
        assert!(actor.buffer.pending(&s));

        // When the session later reaches a genuine assistant answer.
        {
            let mut state = actor.state.write();
            state
                .session_mut_or_create(&s)
                .push_entry(ChatEntry::assistant("recovered"));
        }
        actor
            .handle_phase_changed(&SessionPhaseChanged {
                session_id: s.clone(),
                old_phase: PhaseKind::Streaming,
                new_phase: PhaseKind::Idle,
            })
            .await;

        // Then the retained citation is flushed on the successful turn.
        let cites: Vec<CitationsReceived> = audit.of_type::<CitationsReceived>();
        assert_eq!(cites.len(), 1);
        assert_eq!(cites[0].citations[0].url, "https://example.com/");
    }

    #[tokio::test]
    async fn two_fetches_in_one_turn_accumulate_into_single_flush() {
        // Given a collector with two successful fetches buffered in one turn.
        let (mut actor, audit) = create_actor().await;
        let s = sid();
        for (i, url) in ["https://a.com/", "https://b.com/"].iter().enumerate() {
            let mut cmd = fetch_call(&format!("tc{i}"), url);
            cmd.session_id = s.clone();
            actor.stash_fetch(&cmd.session_id, &cmd.tool_call);
            actor.resolve_pending(&completed(&format!("tc{i}"), "web-fetch", true));
        }
        seed_history(&actor, &s, ChatEntry::assistant("summary"));

        // When the phase transitions Streaming → Idle.
        actor
            .handle_phase_changed(&SessionPhaseChanged {
                session_id: s.clone(),
                old_phase: PhaseKind::Streaming,
                new_phase: PhaseKind::Idle,
            })
            .await;

        // Then one flush carries both URLs.
        let cites: Vec<CitationsReceived> = audit.of_type::<CitationsReceived>();
        assert_eq!(cites.len(), 1);
        assert_eq!(cites[0].citations.len(), 2);
        let urls: Vec<&str> = cites[0].citations.iter().map(|c| c.url.as_str()).collect();
        assert!(urls.contains(&"https://a.com/"));
        assert!(urls.contains(&"https://b.com/"));
    }

    #[tokio::test]
    async fn non_web_tools_are_ignored() {
        // Given a collector with a completion for an unrelated tool.
        let (mut actor, _audit) = create_actor().await;
        let s = sid();
        let mut done = completed("tc1", "run-command", true);
        done.session_id = s.clone();

        // When resolving the unrelated tool completion.
        actor.resolve_pending(&done);

        // Then nothing was buffered.
        assert!(!actor.buffer.pending(&s));
    }
}
