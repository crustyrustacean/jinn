//! Web fetch actor - owns a WebFetcher backend and handles web-fetch tool calls.
//!
//! Subscribes to [`ExecuteWebFetch`] commands dispatched by the tool orchestrator.
//! On startup, registers the `web-fetch` tool definition. On command, parses
//! arguments, delegates to the [`WebFetcher`] backend, and emits
//! [`ToolExecutionCompleted`].
//!
//! # Shutdown
//!
//! Calls [`WebFetcher::shutdown`] during [`Actor::on_stop`] to release
//! resources (e.g., kill a headless browser process).

use std::sync::Arc;

use kameo::actor::ActorRef;
use kameo::prelude::{Context, Message};

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::feat::tools_actor::protocol::command::{ExecuteWebFetch, RegisterTools};
use crate::feat::tools_actor::protocol::event::ToolExecutionCompleted;
use crate::feat::tools_actor::tool_types::{ToolCall, ToolDefinition, ToolResult};
use jinn_web_fetch::{FetchOptions, OutputFormat, WebFetcher};

/// The web fetch actor.
///
/// Owns the chosen [`WebFetcher`] backend and processes `ExecuteWebFetch`
/// commands from the tool orchestrator.
pub struct WebFetchActor {
    deps: ActorDeps,
    web_fetcher: Arc<dyn WebFetcher>,
}

/// Dependencies for [`WebFetchActor`].
pub struct WebFetchActorDeps {
    /// Common actor dependencies (services + bus).
    pub deps: ActorDeps,
    /// The web fetcher backend (e.g., HttpFetcher, HeadlessChromeFetcher).
    pub web_fetcher: Arc<dyn WebFetcher>,
}

/// Arguments parsed from the tool call's JSON arguments string.
#[derive(serde::Deserialize)]
struct WebFetchArgs {
    url: String,
    #[serde(default)]
    options: Option<WebFetchOptionsArgs>,
}

/// The `options` sub-object parsed from tool call arguments.
#[derive(serde::Deserialize, Default)]
struct WebFetchOptionsArgs {
    #[serde(default)]
    format: Option<OutputFormat>,
}

impl kameo::Actor for WebFetchActor {
    type Args = WebFetchActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.recipient::<ExecuteWebFetch>())
            .await;

        // Register the web-fetch tool with the orchestrator.
        let _ = args
            .deps
            .services
            .bus
            .publish(RegisterTools {
                provider: "web-fetch".to_owned(),
                definitions: vec![web_fetch_tool_definition()],
            })
            .await;

        Ok(Self {
            deps: args.deps,
            web_fetcher: args.web_fetcher,
        })
    }

    async fn on_stop(
        &mut self,
        _actor_ref: kameo::actor::WeakActorRef<Self>,
        _reason: kameo::error::ActorStopReason,
    ) -> Result<(), Self::Error> {
        self.web_fetcher.shutdown().await;
        Ok(())
    }
}

impl BusPublish for WebFetchActor {
    fn bus(&self) -> &crate::common::services::bus_service::BusService {
        &self.deps.services.bus
    }
}

impl Message<ExecuteWebFetch> for WebFetchActor {
    type Reply = ();

    async fn handle(&mut self, msg: ExecuteWebFetch, _ctx: &mut Context<Self, Self::Reply>) {
        tracing::trace!(
            tool_call_id = %msg.tool_call.id,
            url_args = %msg.tool_call.arguments,
            "web-fetch: handling ExecuteWebFetch"
        );
        let result = self.execute_fetch(&msg.tool_call).await;
        tracing::info!(
            tool_call_id = %result.tool_call_id,
            success = result.success,
            content_len = result.content.len(),
            "web-fetch: fetch complete"
        );
        let _ = self
            .publish(ToolExecutionCompleted {
                session_id: msg.session_id,
                result,
            })
            .await;
    }
}

impl WebFetchActor {
    /// Parses arguments and executes the fetch.
    async fn execute_fetch(&self, tool_call: &ToolCall) -> ToolResult {
        tracing::debug!(arguments = %tool_call.arguments, "web-fetch: parsing arguments");
        let args = match serde_json::from_str::<WebFetchArgs>(&tool_call.arguments) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(err = %e, "web-fetch: failed to parse arguments");
                return ToolResult {
                    tool_call_id: tool_call.id.clone(),
                    name: tool_call.name.clone(),
                    content: format!("invalid arguments: {e}"),
                    success: false,
                    full_content: None,
                    truncation: None,
                    pin_position: None,
                };
            }
        };

        let format = args.options.and_then(|o| o.format).unwrap_or_default();
        let options = FetchOptions { format };
        tracing::info!(
            url = %args.url,
            format = ?options.format,
            "web-fetch: calling fetcher"
        );

        match self.web_fetcher.fetch(&args.url, options).await {
            Ok(output) => {
                tracing::debug!(
                    status = output.status,
                    content_type = %output.content_type,
                    final_url = %output.url,
                    content_len = output.content.len(),
                    "web-fetch: fetch succeeded"
                );
                ToolResult {
                    tool_call_id: tool_call.id.clone(),
                    name: tool_call.name.clone(),
                    content: output.content,
                    success: true,
                    full_content: None,
                    truncation: None,
                    pin_position: None,
                }
            }
            Err(e) => {
                tracing::warn!(err = %e, "web-fetch: fetch failed");
                ToolResult {
                    tool_call_id: tool_call.id.clone(),
                    name: tool_call.name.clone(),
                    content: format!("fetch failed: {e}"),
                    success: false,
                    full_content: None,
                    truncation: None,
                    pin_position: None,
                }
            }
        }
    }
}

/// Returns the tool definition for `web-fetch`.
fn web_fetch_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "web-fetch".to_owned(),
        description: "Fetch a web page and return its content. Supports multiple output \
            formats (html, text, markdown). Use this to retrieve information from web pages."
            .to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch (http or https)"
                },
                "options": {
                    "type": "object",
                    "description": "Fetch options",
                    "properties": {
                        "format": {
                            "type": "string",
                            "enum": ["html", "text", "markdown"],
                            "description": "Output format. Defaults to 'text'."
                        }
                    }
                }
            },
            "required": ["url"]
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
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use std::sync::Arc;

    use async_trait::async_trait;
    use jinn_web_fetch::{FetchOptions, FetchOutput, WebFetcher};

    use crate::common::bus::test_harness::{TestHarness, await_recorded};
    use crate::feat::tools_actor::protocol::command::ExecuteWebFetch;
    use crate::feat::tools_actor::protocol::command::RegisterTools;
    use crate::feat::tools_actor::protocol::event::ToolExecutionCompleted;
    use crate::feat::tools_actor::tool_types::ToolCall;
    use crate::protocol::SessionId;

    use super::{WebFetchActor, WebFetchActorDeps};
    use kameo::actor::Spawn;

    /// A mock web fetcher that returns a fixed response.
    struct MockFetcher {
        content: String,
        success: bool,
    }

    #[async_trait]
    impl WebFetcher for MockFetcher {
        async fn fetch(
            &self,
            _url: &str,
            _options: FetchOptions,
        ) -> Result<FetchOutput, jinn_web_fetch::FetchError> {
            if self.success {
                Ok(FetchOutput {
                    content: self.content.clone(),
                    url: "https://example.com/".to_owned(),
                    status: 200,
                    content_type: "text/html".to_owned(),
                })
            } else {
                Err(jinn_web_fetch::FetchError::Network)
            }
        }
    }

    fn mock_fetcher_with_success() -> Arc<dyn WebFetcher> {
        Arc::new(MockFetcher {
            content: "Hello, World!".to_owned(),
            success: true,
        })
    }

    fn mock_fetcher_with_error() -> Arc<dyn WebFetcher> {
        Arc::new(MockFetcher {
            content: String::new(),
            success: false,
        })
    }

    #[tokio::test]
    async fn startup_registers_web_fetch_tool() {
        // Given a WebFetchActor.
        let harness = TestHarness::new().await;
        let recorder = harness.spawn_recorder::<RegisterTools>().await;
        let _actor = WebFetchActor::spawn(WebFetchActorDeps {
            deps: harness.actor_deps().await,
            web_fetcher: mock_fetcher_with_success(),
        });

        // Then a RegisterTools command was published.
        let recorded = await_recorded(&recorder, 1, std::time::Duration::from_secs(1)).await;
        assert_eq!(recorded.len(), 1, "should send exactly one RegisterTools");
        assert_eq!(recorded[0].provider, "web-fetch");
        assert_eq!(recorded[0].definitions.len(), 1);
        assert_eq!(recorded[0].definitions[0].name, "web-fetch");
    }

    #[tokio::test]
    async fn execute_web_fetch_success() {
        // Given a WebFetchActor.
        let harness = TestHarness::new().await;
        let recorder = harness.spawn_recorder::<ToolExecutionCompleted>().await;
        let actor = WebFetchActor::spawn(WebFetchActorDeps {
            deps: harness.actor_deps().await,
            web_fetcher: mock_fetcher_with_success(),
        });
        actor.wait_for_startup().await;

        // When sending an ExecuteWebFetch command.
        let session_id = SessionId::new();
        let tool_call = ToolCall {
            id: "tc_1".to_owned(),
            name: "web-fetch".to_owned(),
            arguments: r#"{"url": "https://example.com/"}"#.to_owned(),
        };
        harness
            .publish(ExecuteWebFetch {
                session_id: session_id.clone(),
                tool_call,
            })
            .await;

        // Then a ToolExecutionCompleted event was emitted with success.
        let recorded = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].result.success);
        assert_eq!(recorded[0].result.content, "Hello, World!");
        assert_eq!(recorded[0].session_id, session_id);
    }

    #[tokio::test]
    async fn execute_web_fetch_invalid_args() {
        // Given a WebFetchActor.
        let harness = TestHarness::new().await;
        let recorder = harness.spawn_recorder::<ToolExecutionCompleted>().await;
        let actor = WebFetchActor::spawn(WebFetchActorDeps {
            deps: harness.actor_deps().await,
            web_fetcher: mock_fetcher_with_success(),
        });
        actor.wait_for_startup().await;

        // When sending an ExecuteWebFetch with invalid JSON.
        let session_id = SessionId::new();
        let tool_call = ToolCall {
            id: "tc_2".to_owned(),
            name: "web-fetch".to_owned(),
            arguments: "not json".to_owned(),
        };
        harness
            .publish(ExecuteWebFetch {
                session_id: session_id.clone(),
                tool_call,
            })
            .await;

        // Then a ToolExecutionCompleted event was emitted with failure.
        let recorded = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
        assert_eq!(recorded.len(), 1);
        assert!(!recorded[0].result.success);
        assert!(recorded[0].result.content.contains("invalid arguments"));
    }

    #[tokio::test]
    async fn execute_web_fetch_fetch_error() {
        // Given a WebFetchActor with an error-producing fetcher.
        let harness = TestHarness::new().await;
        let recorder = harness.spawn_recorder::<ToolExecutionCompleted>().await;
        let actor = WebFetchActor::spawn(WebFetchActorDeps {
            deps: harness.actor_deps().await,
            web_fetcher: mock_fetcher_with_error(),
        });
        actor.wait_for_startup().await;

        // When sending a valid ExecuteWebFetch.
        let session_id = SessionId::new();
        let tool_call = ToolCall {
            id: "tc_3".to_owned(),
            name: "web-fetch".to_owned(),
            arguments: r#"{"url": "https://example.com/"}"#.to_owned(),
        };
        harness
            .publish(ExecuteWebFetch {
                session_id: session_id.clone(),
                tool_call,
            })
            .await;

        // Then a ToolExecutionCompleted event was emitted with failure.
        let recorded = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
        assert_eq!(recorded.len(), 1);
        assert!(!recorded[0].result.success);
        assert!(recorded[0].result.content.contains("fetch failed"));
    }
}
