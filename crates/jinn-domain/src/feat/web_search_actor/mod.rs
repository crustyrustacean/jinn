//! Web search actor — owns a [`WebSearcher`] backend and handles the
//! `web-search` tool calls.
//!
//! Subscribes to [`ExecuteWebSearch`] commands dispatched by the tool
//! orchestrator. On startup, registers the `web-search` tool definition. On
//! command, parses arguments, delegates to the [`WebSearcher`] backend
//! (currently [`DdgSearcher`]), and emits [`ToolExecutionCompleted`].
//!
//! Unlike `web-fetch`, the search backend is fixed to DuckDuckGo (via
//! [`DdgSearcher`]) — there is no backend-selection knob. Configuration is
//! limited to result count, region, and safe search.
//!
//! # Shutdown
//!
//! Stateless — no resources to release during [`Actor::on_stop`].

use std::sync::Arc;

use jinn_web_search::{SearchOptions, WebSearcher};
use kameo::actor::ActorRef;
use kameo::prelude::{Context, Message};

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::feat::tools_actor::protocol::command::{ExecuteWebSearch, RegisterTools};
use crate::feat::tools_actor::protocol::event::{
    ToolExecutionCompleted, ToolExecutionOutput, ToolExecutionStarted, ToolOutputKind,
};
use crate::feat::tools_actor::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::protocol::SessionId;

use serde::{Deserialize, Serialize};

/// Web search tool configuration.
///
/// Serialized as `[web_search]` in `jinn.toml`.
/// Controls the behavior of the `web-search` tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebSearchConfig {
    /// Browser backend for web search. Default: `"http"`.
    ///
    /// `http` uses plain reqwest; `headless-chrome` and `headed-chrome` drive
    /// the shared browser. Browser launch settings come from the shared
    /// `[browser]` table.
    #[serde(default = "default_web_search_backend")]
    pub backend: WebSearchBackend,
    /// Maximum number of results to return per search. Default: `10`.
    #[serde(default = "default_max_results")]
    pub max_results: usize,
    /// DuckDuckGo region code, e.g. `"wt-wt"` (global) or `"us-en"`.
    /// Default: `"wt-wt"`.
    #[serde(default = "default_region")]
    pub region: String,
    /// Whether safe search is on. Default: `true`.
    #[serde(default = "default_safe_search")]
    pub safe_search: bool,
}

fn default_web_search_backend() -> WebSearchBackend {
    WebSearchBackend::Http
}

fn default_max_results() -> usize {
    10
}

fn default_region() -> String {
    "wt-wt".to_owned()
}

fn default_safe_search() -> bool {
    true
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            backend: default_web_search_backend(),
            max_results: default_max_results(),
            region: default_region(),
            safe_search: default_safe_search(),
        }
    }
}

/// Alias for the shared browser backend selector used by web search.
///
/// `web-search` selects `http` (default), `headless-chrome`, or
/// `headed-chrome`. This mirrors `web-fetch`'s backend via the shared
/// [`crate::feat::browser::BrowserBackend`].
pub type WebSearchBackend = crate::feat::browser::BrowserBackend;

/// The web search actor.
///
/// Owns a [`WebSearcher`] backend and processes [`ExecuteWebSearch`] commands
/// from the tool orchestrator. Each call is dispatched to a standalone tokio
/// task so the mailbox stays free for concurrent requests.
pub struct WebSearchActor {
    deps: ActorDeps,
    web_searcher: Arc<dyn WebSearcher>,
    config: WebSearchConfig,
}

/// Dependencies for [`WebSearchActor`].
#[derive(Clone)]
pub struct WebSearchActorDeps {
    /// Common actor dependencies (services + bus).
    pub deps: ActorDeps,
    /// The web searcher backend (e.g. `DdgSearcher`).
    pub web_searcher: Arc<dyn WebSearcher>,
    /// Tool configuration loaded from `[web_search]` in `jinn.toml`.
    pub config: WebSearchConfig,
}

/// Arguments parsed from the tool call's JSON arguments string.
#[derive(serde::Deserialize)]
struct WebSearchArgs {
    query: String,
    /// Optional per-call override; the actor applies the lower of this and the
    /// configured `max_results`.
    #[serde(default)]
    max_results: Option<usize>,
}

impl kameo::Actor for WebSearchActor {
    type Args = WebSearchActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.recipient::<ExecuteWebSearch>())
            .await;

        // Register the web-search tool with the orchestrator.
        let () = args
            .deps
            .services
            .bus
            .publish(RegisterTools {
                provider: "web-search".to_owned(),
                definitions: vec![web_search_tool_definition()],
                session_id: None,
            })
            .await;

        Ok(Self {
            deps: args.deps,
            web_searcher: args.web_searcher,
            config: args.config,
        })
    }
}

impl BusPublish for WebSearchActor {
    fn bus(&self) -> &crate::common::services::bus_service::BusService {
        &self.deps.services.bus
    }
}

impl Message<ExecuteWebSearch> for WebSearchActor {
    type Reply = ();

    async fn handle(&mut self, msg: ExecuteWebSearch, _ctx: &mut Context<Self, Self::Reply>) {
        tracing::trace!(
            tool_call_id = %msg.tool_call.id,
            query_args = %msg.tool_call.arguments,
            "web-search: handling ExecuteWebSearch"
        );
        // Dispatch the search to a standalone task and return immediately. The
        // mailbox is freed for the next request, so concurrent searches run as
        // independent tasks instead of serially blocking the actor.
        let web_searcher = self.web_searcher.clone();
        let bus = self.deps.services.bus.clone();
        let config = self.config.clone();
        let tool_call = msg.tool_call;
        let session_id = msg.session_id;
        let dispatched_at = msg.dispatched_at;
        tokio::spawn(async move {
            let result = execute_search(
                &web_searcher,
                &config,
                &tool_call,
                &session_id,
                dispatched_at,
                &bus,
            )
            .await;
            tracing::info!(
                tool_call_id = %result.tool_call_id,
                success = result.success,
                content_len = result.content.len(),
                "web-search: search complete"
            );
            let () = bus
                .publish(ToolExecutionCompleted { session_id, result })
                .await;
        });
    }
}

/// Parses arguments and executes the search.
async fn execute_search(
    web_searcher: &Arc<dyn WebSearcher>,
    config: &WebSearchConfig,
    tool_call: &ToolCall,
    session_id: &SessionId,
    dispatched_at: jiff::Timestamp,
    bus: &crate::common::services::bus_service::BusService,
) -> ToolResult {
    tracing::debug!(arguments = %tool_call.arguments, "web-search: parsing arguments");
    let args = match serde_json::from_str::<WebSearchArgs>(&tool_call.arguments) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(err = %e, "web-search: failed to parse arguments");
            return failure_result(tool_call, format!("invalid arguments: {e}"));
        }
    };

    if args.query.trim().is_empty() {
        tracing::warn!("web-search: empty query rejected");
        return failure_result(tool_call, "query must not be empty".to_owned());
    }

    // Apply the lower of the per-call override and the configured max.
    let max_results = args
        .max_results
        .map_or(config.max_results, |m| m.min(config.max_results));
    let options = SearchOptions {
        max_results,
        region: config.region.clone(),
        safe_search: config.safe_search,
    };

    tracing::info!(query = %args.query, max_results, "web-search: calling searcher");
    // Lazy streaming: the pending ToolResult entry is only created when the
    // searcher actually reports a wait (challenge detection/human-wait ticks).
    // Clean searches emit nothing and complete exactly as before.
    let started = std::sync::atomic::AtomicBool::new(false);
    let on_event: jinn_web_fetch::ProgressFn = std::sync::Arc::new({
        let bus = bus.clone();
        let session_id = session_id.clone();
        let tool_call_id = tool_call.id.clone();
        let name = tool_call.name.clone();
        move |progress: jinn_web_fetch::RenderProgress| {
            let text = describe_progress(&progress);
            // The observer runs inside a `spawn_blocking` render; re-enter the
            // async runtime to publish onto the bus.
            let handle = tokio::runtime::Handle::current();
            if !started.swap(true, std::sync::atomic::Ordering::SeqCst) {
                handle.block_on(bus.publish(ToolExecutionStarted {
                    session_id: session_id.clone(),
                    tool_call_id: tool_call_id.clone(),
                    name: name.clone(),
                    dispatched_at,
                }));
            }
            handle.block_on(bus.publish(ToolExecutionOutput {
                session_id: session_id.clone(),
                tool_call_id: tool_call_id.clone(),
                output: text,
                kind: ToolOutputKind::Alert,
            }));
        }
    });
    match web_searcher
        .search_observed(&args.query, &options, on_event)
        .await
    {
        Ok(results) => {
            let content = format_results(&results);
            ToolResult {
                tool_call_id: tool_call.id.clone(),
                name: tool_call.name.clone(),
                content,
                success: true,
                full_content: None,
                truncation: None,
                pin_position: None,
            }
        }
        Err(e) => {
            tracing::warn!(err = %e, "web-search: search failed");
            failure_result(tool_call, format!("search failed: {e}"))
        }
    }
}

/// Renders a [`RenderProgress`] event as user-facing streaming text.
fn describe_progress(progress: &jinn_web_fetch::RenderProgress) -> String {
    match progress {
        jinn_web_fetch::RenderProgress::ChallengeDetected { kind, url } => {
            format!("⚠ bot challenge detected ({kind:?}) at {url} — solve it in the browser window; waiting for you…")
        }
        jinn_web_fetch::RenderProgress::WaitingForHuman { elapsed_secs } => {
            format!("still waiting for the challenge to clear ({elapsed_secs}s elapsed)")
        }
    }
}

/// Builds a failed [`ToolResult`] with the given error message.
fn failure_result(tool_call: &ToolCall, message: String) -> ToolResult {
    ToolResult {
        tool_call_id: tool_call.id.clone(),
        name: tool_call.name.clone(),
        content: message,
        success: false,
        full_content: None,
        truncation: None,
        pin_position: None,
    }
}

/// Formats results as compact numbered text, one result per block:
///
/// ```text
/// 1. Title — url
///    snippet
/// ```
fn format_results(results: &[jinn_web_search::SearchResult]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for (i, r) in results.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        let _ = write!(out, "{}. {} — {}\n   {}", i + 1, r.title, r.url, r.snippet);
    }
    if results.is_empty() {
        out.push_str("No results found.");
    }
    out
}

/// Returns the tool definition for `web-search`.
fn web_search_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "web-search".to_owned(),
        description: "Search the web via DuckDuckGo and return the top results (title, URL, \
            snippet). Works with any provider. Use this to find current information; follow up \
            with the web-fetch tool for full page content."
            .to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum results to return (capped by configuration). Defaults to configured value."
                }
            },
            "required": ["query"]
        }),
        prompt_snippet: None,
        prompt_guidelines: vec![],
        server_tool_type: None,
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

    #[test]
    fn default_config_has_expected_values() {
        // Given no input.
        // When constructing the default config.
        let config = WebSearchConfig::default();

        // Then the documented defaults are used.
        assert_eq!(config.max_results, 10);
        assert_eq!(config.region, "wt-wt");
        assert!(config.safe_search);
    }

    #[test]
    fn config_serializes_with_web_search_section() {
        // Given a default config.
        let config = WebSearchConfig::default();

        // When serializing to TOML.
        let toml = toml::to_string(&config).expect("serialize");

        // Then all three fields appear.
        assert!(toml.contains("max_results = 10"));
        assert!(toml.contains("region = \"wt-wt\""));
        assert!(toml.contains("safe_search = true"));
    }

    #[test]
    fn config_round_trips_through_toml() {
        // Given a custom config.
        let config = WebSearchConfig {
            backend: WebSearchBackend::HeadlessChrome,
            max_results: 5,
            region: "us-en".to_owned(),
            safe_search: false,
        };

        // When serializing then deserializing.
        let toml = toml::to_string(&config).expect("serialize");
        let back: WebSearchConfig = toml::from_str(&toml).expect("deserialize");

        // Then the custom values survive.
        assert_eq!(back, config);
    }

    #[test]
    fn config_uses_defaults_when_empty() {
        // Given an empty TOML table.
        let toml = "";

        // When deserializing.
        let config: WebSearchConfig = toml::from_str(toml).expect("deserialize");

        // Then defaults are filled in.
        assert_eq!(config, WebSearchConfig::default());
    }

    #[test]
    fn format_results_empty_returns_no_results_message() {
        // Given no results.
        // When formatting.
        let out = format_results(&[]);

        // Then a no-results placeholder is shown.
        assert_eq!(out, "No results found.");
    }

    #[test]
    fn format_results_single_result_is_numbered() {
        // Given one result.
        let results = vec![jinn_web_search::SearchResult {
            title: "Example".to_owned(),
            url: "https://example.com".to_owned(),
            snippet: "An example site".to_owned(),
        }];

        // When formatting.
        let out = format_results(&results);

        // Then it is numbered 1 with title, url, snippet.
        assert_eq!(out, "1. Example — https://example.com\n   An example site");
    }

    #[test]
    fn format_results_multiple_results_are_separated() {
        // Given two results.
        let results = vec![
            jinn_web_search::SearchResult {
                title: "First".to_owned(),
                url: "https://first.com".to_owned(),
                snippet: "first snippet".to_owned(),
            },
            jinn_web_search::SearchResult {
                title: "Second".to_owned(),
                url: "https://second.com".to_owned(),
                snippet: "second snippet".to_owned(),
            },
        ];

        // When formatting.
        let out = format_results(&results);

        // Then both appear, numbered, separated by a blank line.
        assert_eq!(
            out,
            "1. First — https://first.com\n   first snippet\n\n2. Second — https://second.com\n   second snippet"
        );
    }
}

#[cfg(test)]
mod actor_tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "test assertions"
    )]
    use super::{WebSearchActor, WebSearchActorDeps, WebSearchConfig};
    use crate::common::bus::test_harness::{TestHarness, await_recorded};
    use crate::feat::tools_actor::protocol::command::{ExecuteWebSearch, RegisterTools};
    use crate::feat::tools_actor::protocol::event::{
        ToolExecutionCompleted, ToolExecutionOutput, ToolExecutionStarted,
    };
    use crate::feat::tools_actor::tool_types::ToolCall;
    use crate::protocol::SessionId;
    use async_trait::async_trait;
    use jinn_web_search::{SearchError, SearchOptions, SearchResult, WebSearcher};
    use kameo::actor::Spawn;
    use std::sync::{Arc, Mutex};

    /// A mock searcher that returns canned results or an error.
    struct MockSearcher {
        results: Vec<SearchResult>,
        fail: bool,
        captured_query: Arc<Mutex<Option<String>>>,
        /// When set, `search_observed` fires this progress event before resolving.
        progress_event: Option<jinn_web_fetch::RenderProgress>,
    }

    #[async_trait]
    impl WebSearcher for MockSearcher {
        async fn search(
            &self,
            query: &str,
            options: &SearchOptions,
        ) -> Result<Vec<SearchResult>, SearchError> {
            let noop: jinn_web_fetch::ProgressFn = Arc::new(|_| {});
            self.search_observed(query, options, noop).await
        }

        async fn search_observed(
            &self,
            query: &str,
            _options: &SearchOptions,
            on_event: jinn_web_fetch::ProgressFn,
        ) -> Result<Vec<SearchResult>, SearchError> {
            *self.captured_query.lock().expect("lock") = Some(query.to_owned());
            if let Some(progress) = &self.progress_event {
                // Faithful to production: the observer runs inside the
                // searcher's spawn_blocking render, never on the async worker.
                let progress = progress.clone();
                let on_event = on_event.clone();
                tokio::task::spawn_blocking(move || on_event(progress))
                    .await
                    .expect("observer task");
            }
            if self.fail {
                Err(SearchError::Network)
            } else {
                Ok(self.results.clone())
            }
        }
    }

    fn mock_results() -> Vec<SearchResult> {
        vec![
            SearchResult {
                title: "Rust Programming".to_owned(),
                url: "https://rust-lang.org".to_owned(),
                snippet: "A language empowering everyone".to_owned(),
            },
            SearchResult {
                title: "Learn Rust".to_owned(),
                url: "https://doc.rust-lang.org".to_owned(),
                snippet: "The Rust book".to_owned(),
            },
        ]
    }

    fn default_config() -> WebSearchConfig {
        WebSearchConfig::default()
    }

    #[tokio::test]
    async fn startup_registers_web_search_tool() {
        // Given a WebSearchActor.
        let harness = TestHarness::new().await;
        let recorder = harness.spawn_recorder::<RegisterTools>().await;
        let _actor = WebSearchActor::spawn(WebSearchActorDeps {
            deps: harness.actor_deps().await,
            web_searcher: Arc::new(MockSearcher {
                results: mock_results(),
                fail: false,
                captured_query: Arc::new(Mutex::new(None)),
                progress_event: None,
            }),
            config: default_config(),
        });

        // Then a RegisterTools command was published.
        let messages = await_recorded(&recorder, 1, std::time::Duration::from_secs(1)).await;
        assert_eq!(messages.len(), 1, "should send exactly one RegisterTools");
        assert_eq!(messages[0].provider, "web-search");
        assert_eq!(messages[0].definitions.len(), 1);
        assert_eq!(messages[0].definitions[0].name, "web-search");
    }

    #[tokio::test]
    async fn execute_web_search_success() {
        // Given a WebSearchActor.
        let harness = TestHarness::new().await;
        let recorder = harness.spawn_recorder::<ToolExecutionCompleted>().await;
        let actor = WebSearchActor::spawn(WebSearchActorDeps {
            deps: harness.actor_deps().await,
            web_searcher: Arc::new(MockSearcher {
                results: mock_results(),
                fail: false,
                captured_query: Arc::new(Mutex::new(None)),
                progress_event: None,
            }),
            config: default_config(),
        });
        actor.wait_for_startup().await;

        // When sending an ExecuteWebSearch command.
        let session_id = SessionId::new();
        let tool_call = ToolCall {
            id: "tc_1".to_owned(),
            name: "web-search".to_owned(),
            arguments: r#"{"query": "rust"}"#.to_owned(),
        };
        harness
            .publish(ExecuteWebSearch {
                session_id: session_id.clone(),
                tool_call,
                dispatched_at: jiff::Timestamp::now(),
            })
            .await;

        // Then a ToolExecutionCompleted event was emitted with success.
        let messages = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
        assert_eq!(messages.len(), 1);
        assert!(messages[0].result.success);
        assert_eq!(messages[0].session_id, session_id);
    }

    #[tokio::test]
    async fn execute_web_search_invalid_args() {
        // Given a WebSearchActor.
        let harness = TestHarness::new().await;
        let recorder = harness.spawn_recorder::<ToolExecutionCompleted>().await;
        let actor = WebSearchActor::spawn(WebSearchActorDeps {
            deps: harness.actor_deps().await,
            web_searcher: Arc::new(MockSearcher {
                results: mock_results(),
                fail: false,
                captured_query: Arc::new(Mutex::new(None)),
                progress_event: None,
            }),
            config: default_config(),
        });
        actor.wait_for_startup().await;

        // When sending an ExecuteWebSearch with invalid JSON.
        let session_id = SessionId::new();
        let tool_call = ToolCall {
            id: "tc_2".to_owned(),
            name: "web-search".to_owned(),
            arguments: "not json".to_owned(),
        };
        harness
            .publish(ExecuteWebSearch {
                session_id,
                tool_call,
                dispatched_at: jiff::Timestamp::now(),
            })
            .await;

        // Then a ToolExecutionCompleted event was emitted with failure.
        let messages = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
        assert_eq!(messages.len(), 1);
        assert!(!messages[0].result.success);
        assert!(messages[0].result.content.contains("invalid arguments"));
    }

    #[tokio::test]
    async fn execute_web_search_empty_query() {
        // Given a WebSearchActor.
        let harness = TestHarness::new().await;
        let recorder = harness.spawn_recorder::<ToolExecutionCompleted>().await;
        let actor = WebSearchActor::spawn(WebSearchActorDeps {
            deps: harness.actor_deps().await,
            web_searcher: Arc::new(MockSearcher {
                results: mock_results(),
                fail: false,
                captured_query: Arc::new(Mutex::new(None)),
                progress_event: None,
            }),
            config: default_config(),
        });
        actor.wait_for_startup().await;

        // When sending an ExecuteWebSearch with an empty query.
        let session_id = SessionId::new();
        let tool_call = ToolCall {
            id: "tc_3".to_owned(),
            name: "web-search".to_owned(),
            arguments: r#"{"query": "   "}"#.to_owned(),
        };
        harness
            .publish(ExecuteWebSearch {
                session_id,
                tool_call,
                dispatched_at: jiff::Timestamp::now(),
            })
            .await;

        // Then a ToolExecutionCompleted event was emitted with failure.
        let messages = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
        assert_eq!(messages.len(), 1);
        assert!(!messages[0].result.success);
        assert!(messages[0].result.content.contains("empty"));
    }

    #[tokio::test]
    async fn execute_web_search_formats_numbered_text() {
        // Given a WebSearchActor whose searcher returns two canned results.
        let harness = TestHarness::new().await;
        let recorder = harness.spawn_recorder::<ToolExecutionCompleted>().await;
        let actor = WebSearchActor::spawn(WebSearchActorDeps {
            deps: harness.actor_deps().await,
            web_searcher: Arc::new(MockSearcher {
                results: mock_results(),
                fail: false,
                captured_query: Arc::new(Mutex::new(None)),
                progress_event: None,
            }),
            config: default_config(),
        });
        actor.wait_for_startup().await;

        // When sending a valid ExecuteWebSearch.
        let session_id = SessionId::new();
        let tool_call = ToolCall {
            id: "tc_4".to_owned(),
            name: "web-search".to_owned(),
            arguments: r#"{"query": "rust"}"#.to_owned(),
        };
        harness
            .publish(ExecuteWebSearch {
                session_id,
                tool_call,
                dispatched_at: jiff::Timestamp::now(),
            })
            .await;

        // Then the result is formatted as numbered text with title, url, snippet.
        let messages = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
        assert_eq!(messages.len(), 1);
        assert!(messages[0].result.success);
        assert!(
            messages[0]
                .result
                .content
                .contains("1. Rust Programming — https://rust-lang.org")
        );
        assert!(
            messages[0]
                .result
                .content
                .contains("2. Learn Rust — https://doc.rust-lang.org")
        );
    }

    #[tokio::test]
    async fn challenge_progress_emits_started_then_output_then_completed() {
        // Given a searcher that reports a challenge detection then succeeds.
        let harness = TestHarness::new().await;
        let started_rec = harness.spawn_recorder::<ToolExecutionStarted>().await;
        let output_rec = harness.spawn_recorder::<ToolExecutionOutput>().await;
        let completed_rec = harness.spawn_recorder::<ToolExecutionCompleted>().await;
        let actor = WebSearchActor::spawn(WebSearchActorDeps {
            deps: harness.actor_deps().await,
            web_searcher: Arc::new(MockSearcher {
                results: mock_results(),
                fail: false,
                captured_query: Arc::new(Mutex::new(None)),
                progress_event: Some(jinn_web_fetch::RenderProgress::ChallengeDetected {
                    kind: jinn_web_fetch::challenge::ChallengeKind::DdgAnomaly,
                    url: "https://html.duckduckgo.com/html".to_owned(),
                }),
            }),
            config: default_config(),
        });
        actor.wait_for_startup().await;

        // When sending a search whose render reports a challenge.
        let session_id = SessionId::new();
        let tool_call = ToolCall {
            id: "tc_alert".to_owned(),
            name: "web-search".to_owned(),
            arguments: r#"{"query": "rust"}"#.to_owned(),
        };
        harness
            .publish(ExecuteWebSearch {
                session_id: session_id.clone(),
                tool_call,
                dispatched_at: jiff::Timestamp::now(),
            })
            .await;

        // Then exactly one Started, one Output, and one Completed were emitted.
        let started = await_recorded(&started_rec, 1, std::time::Duration::from_secs(2)).await;
        let output = await_recorded(&output_rec, 1, std::time::Duration::from_secs(2)).await;
        let completed = await_recorded(&completed_rec, 1, std::time::Duration::from_secs(2)).await;
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].tool_call_id, "tc_alert");
        assert_eq!(output.len(), 1);
        assert!(output[0].output.contains("bot challenge detected"));
        assert_eq!(completed.len(), 1);
        assert!(completed[0].result.success);
    }

    #[tokio::test]
    async fn clean_search_emits_no_tool_execution_started() {
        // Given a searcher that reports no progress events.
        let harness = TestHarness::new().await;
        let started_rec = harness.spawn_recorder::<ToolExecutionStarted>().await;
        let completed_rec = harness.spawn_recorder::<ToolExecutionCompleted>().await;
        let actor = WebSearchActor::spawn(WebSearchActorDeps {
            deps: harness.actor_deps().await,
            web_searcher: Arc::new(MockSearcher {
                results: mock_results(),
                fail: false,
                captured_query: Arc::new(Mutex::new(None)),
                progress_event: None,
            }),
            config: default_config(),
        });
        actor.wait_for_startup().await;

        // When sending a clean search.
        harness
            .publish(ExecuteWebSearch {
                session_id: SessionId::new(),
                tool_call: ToolCall {
                    id: "tc_clean".to_owned(),
                    name: "web-search".to_owned(),
                    arguments: r#"{"query": "rust"}"#.to_owned(),
                },
                dispatched_at: jiff::Timestamp::now(),
            })
            .await;

        // Then it completes but never emits ToolExecutionStarted.
        let completed = await_recorded(&completed_rec, 1, std::time::Duration::from_secs(2)).await;
        assert_eq!(completed.len(), 1);
        assert!(
            await_recorded(&started_rec, 0, std::time::Duration::from_millis(300))
                .await
                .is_empty()
        );
    }
}
