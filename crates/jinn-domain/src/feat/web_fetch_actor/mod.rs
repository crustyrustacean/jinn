//! Web fetch actor - owns a WebFetcher backend and handles web-fetch tool calls.
//!
//! Subscribes to [`ExecuteWebFetch`] commands dispatched by the tool orchestrator.
//! On activation, registers the `web-fetch` tool definition. On command, parses
//! arguments, delegates to the [`WebFetcher`] backend, and emits
//! [`ToolExecutionCompleted`].
//!
//! # Shutdown
//!
//! Calls [`WebFetcher::shutdown`] during [`Actor::on_shutdown`] to release
//! resources (e.g., kill a headless browser process).

use std::sync::Arc;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::feat::tools_actor::protocol::command::{ExecuteWebFetch, RegisterTools};
use crate::feat::tools_actor::protocol::event::ToolExecutionCompleted;
use crate::feat::tools_actor::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::protocol::{Command, Event};
use jinn_web_fetch::{FetchOptions, OutputFormat, WebFetcher};

/// The web fetch actor.
///
/// Owns the chosen [`WebFetcher`] backend and processes `ExecuteWebFetch`
/// commands from the tool orchestrator.
pub struct WebFetchActor {
    web_fetcher: Arc<dyn WebFetcher>,
}

/// Dependencies for [`WebFetchActor`].
pub struct WebFetchActorDeps {
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

impl Actor for WebFetchActor {
    type Message = NoDirectMsg;
    type Deps = WebFetchActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<ExecuteWebFetch>();
        ctx.set_description("Fetches web pages via configured backend");

        // Register the web-fetch tool with the orchestrator.
        if let Err(e) = ctx.send_command(Command::RegisterTools(RegisterTools {
            provider: "web-fetch".to_owned(),
            definitions: vec![web_fetch_tool_definition()],
        })) {
            tracing::warn!(err = ?e, "web-fetch actor failed to register tools");
        }

        Self {
            web_fetcher: deps.web_fetcher,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(cmd) => self.handle_command(&cmd, ctx).await,
            _ => {}
        }
    }

    async fn on_shutdown(&mut self, _ctx: &ActorContext) {
        self.web_fetcher.shutdown().await;
    }
}

impl WebFetchActor {
    /// Dispatches a command to the appropriate handler.
    async fn handle_command(&mut self, cmd: &Command, ctx: &ActorContext) {
        match cmd {
            Command::ExecuteWebFetch(payload) => {
                self.handle_execute_web_fetch(payload, ctx).await;
            }
            _ => {}
        }
    }

    /// Handles an `ExecuteWebFetch` command.
    async fn handle_execute_web_fetch(&self, payload: &ExecuteWebFetch, ctx: &ActorContext) {
        tracing::trace!(
            tool_call_id = %payload.tool_call.id,
            url_args = %payload.tool_call.arguments,
            "web-fetch: handling ExecuteWebFetch"
        );
        let result = self.execute_fetch(&payload.tool_call).await;
        tracing::info!(
            tool_call_id = %result.tool_call_id,
            success = result.success,
            content_len = result.content.len(),
            "web-fetch: fetch complete"
        );
        if let Err(e) = ctx.send_event(Event::ToolExecutionCompleted(ToolExecutionCompleted {
            session_id: payload.session_id.clone(),
            result,
        })) {
            tracing::warn!(
                err = ?e,
                "web-fetch actor failed to send ToolExecutionCompleted"
            );
        }
    }

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
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use std::sync::Arc;

    use async_trait::async_trait;
    use jinn_web_fetch::{FetchOptions, FetchOutput, WebFetcher};

    use crate::common::actor::{Actor, ActorContext, ActorEnvelope, RecordingSink};
    use crate::feat::tools_actor::protocol::command::ExecuteWebFetch;
    use crate::feat::tools_actor::tool_types::ToolCall;
    use crate::protocol::{Command, Event, SessionId};

    use super::{WebFetchActor, WebFetchActorDeps};

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

    fn test_context() -> (Arc<RecordingSink>, ActorContext) {
        let sink = Arc::new(RecordingSink::new());
        let ctx = ActorContext::new("test-web-fetch", sink.clone());
        (sink, ctx)
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

    #[rstest::rstest]
    #[tokio::test]
    async fn activate_registers_web_fetch_tool() {
        // Given a WebFetchActor with a mock fetcher.
        let (sink, mut ctx) = test_context();
        let _actor = WebFetchActor::activate(
            WebFetchActorDeps {
                web_fetcher: mock_fetcher_with_success(),
            },
            &mut ctx,
        );

        // Then a RegisterTools command was emitted.
        let commands = sink.take_commands();
        assert_eq!(
            commands.len(),
            1,
            "should send exactly one RegisterTools command"
        );
        let cmd = &commands[0];
        match cmd {
            Command::RegisterTools(reg) => {
                assert_eq!(reg.provider, "web-fetch");
                assert_eq!(reg.definitions.len(), 1);
                assert_eq!(reg.definitions[0].name, "web-fetch");
            }
            other => panic!("expected RegisterTools, got {other:?}"),
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_web_fetch_success() {
        // Given an activated WebFetchActor.
        let (sink, mut ctx) = test_context();
        let mut actor = WebFetchActor::activate(
            WebFetchActorDeps {
                web_fetcher: mock_fetcher_with_success(),
            },
            &mut ctx,
        );

        // Clear the RegisterTools command from activation.
        sink.take_commands();

        // When sending an ExecuteWebFetch command.
        let session_id = SessionId::new();
        let tool_call = ToolCall {
            id: "tc_1".to_owned(),
            name: "web-fetch".to_owned(),
            arguments: r#"{"url": "https://example.com/"}"#.to_owned(),
        };
        actor
            .handle(
                ActorEnvelope::Command(Command::ExecuteWebFetch(ExecuteWebFetch {
                    session_id: session_id.clone(),
                    tool_call,
                })),
                &ctx,
            )
            .await;

        // Then a ToolExecutionCompleted event was emitted with success.
        let events = sink.take_events();
        let completed: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::ToolExecutionCompleted(c) => Some(c.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(completed.len(), 1);
        assert!(completed[0].result.success);
        assert_eq!(completed[0].result.content, "Hello, World!");
        assert_eq!(completed[0].session_id, session_id);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_web_fetch_invalid_args() {
        // Given an activated WebFetchActor.
        let (sink, mut ctx) = test_context();
        let mut actor = WebFetchActor::activate(
            WebFetchActorDeps {
                web_fetcher: mock_fetcher_with_success(),
            },
            &mut ctx,
        );

        sink.take_commands();

        // When sending an ExecuteWebFetch with invalid JSON.
        let session_id = SessionId::new();
        let tool_call = ToolCall {
            id: "tc_2".to_owned(),
            name: "web-fetch".to_owned(),
            arguments: "not json".to_owned(),
        };
        actor
            .handle(
                ActorEnvelope::Command(Command::ExecuteWebFetch(ExecuteWebFetch {
                    session_id: session_id.clone(),
                    tool_call,
                })),
                &ctx,
            )
            .await;

        // Then a ToolExecutionCompleted event was emitted with failure.
        let events = sink.take_events();
        let completed: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::ToolExecutionCompleted(c) => Some(c.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(completed.len(), 1);
        assert!(!completed[0].result.success);
        assert!(completed[0].result.content.contains("invalid arguments"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_web_fetch_fetch_error() {
        // Given an activated WebFetchActor with an error-producing fetcher.
        let (sink, mut ctx) = test_context();
        let mut actor = WebFetchActor::activate(
            WebFetchActorDeps {
                web_fetcher: mock_fetcher_with_error(),
            },
            &mut ctx,
        );

        sink.take_commands();

        // When sending a valid ExecuteWebFetch.
        let session_id = SessionId::new();
        let tool_call = ToolCall {
            id: "tc_3".to_owned(),
            name: "web-fetch".to_owned(),
            arguments: r#"{"url": "https://example.com/"}"#.to_owned(),
        };
        actor
            .handle(
                ActorEnvelope::Command(Command::ExecuteWebFetch(ExecuteWebFetch {
                    session_id: session_id.clone(),
                    tool_call,
                })),
                &ctx,
            )
            .await;

        // Then a ToolExecutionCompleted event was emitted with failure.
        let events = sink.take_events();
        let completed: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::ToolExecutionCompleted(c) => Some(c.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(completed.len(), 1);
        assert!(!completed[0].result.success);
        assert!(completed[0].result.content.contains("fetch failed"));
    }
}
