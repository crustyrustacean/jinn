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

use serde::{Deserialize, Serialize};

use crate::feat::browser::BrowserBackend;

/// Web fetch backend selection.
///
/// Determines which fetching strategy is used for the `web-fetch` tool.
/// Selected once at startup from `jinn.toml` and never changes at runtime.
///
/// This is the shared [`crate::feat::browser::BrowserBackend`]; the
/// `http`/`headless-chrome`/`headed-chrome` variants are common to both
/// `web-fetch` and `web-search`. `headless-chrome` is the default for
/// `web-fetch` (see [`WebFetchConfig::default`]).
pub type WebFetchBackend = crate::feat::browser::BrowserBackend;

/// Web fetch tool configuration.
///
/// Serialized as `[web_fetch]` in `jinn.toml`.
/// Controls which backend the `web-fetch` tool uses. Browser launch settings
/// (binary, user agent, challenge timeout) live in the shared `[browser]`
/// table and are not duplicated here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebFetchConfig {
    /// The backend to use for web fetching. Default: `"headless-chrome"`.
    #[serde(default = "default_web_fetch_backend")]
    pub backend: WebFetchBackend,
}

/// The default `web-fetch` backend is headless Chrome.
fn default_web_fetch_backend() -> WebFetchBackend {
    BrowserBackend::HeadlessChrome
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            backend: default_web_fetch_backend(),
        }
    }
}

use std::sync::Arc;

use kameo::actor::ActorRef;
use kameo::prelude::{Context, Message};

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::feat::tools_actor::protocol::command::{ExecuteWebFetch, RegisterTools};
use crate::feat::tools_actor::protocol::event::{
    ToolExecutionCompleted, ToolExecutionOutput, ToolExecutionStarted,
};
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
#[derive(Clone)]
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
        let () = args
            .deps
            .services
            .bus
            .publish(RegisterTools {
                provider: "web-fetch".to_owned(),
                definitions: vec![web_fetch_tool_definition()],
                session_id: None,
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
        // Dispatch the fetch to a standalone task and return immediately. The
        // mailbox is freed for the next request, so concurrent fetches (across
        // sessions, or multiple URLs in one tool batch) run as independent
        // Chrome tabs instead of serially blocking the actor. The task runs the
        // fetch to completion and publishes the result itself.
        let web_fetcher = self.web_fetcher.clone();
        let bus = self.deps.services.bus.clone();
        let tool_call = msg.tool_call;
        let session_id = msg.session_id;
        let dispatched_at = msg.dispatched_at;
        tokio::spawn(async move {
            let result = execute_fetch(
                &web_fetcher,
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
                "web-fetch: fetch complete"
            );
            let () = bus
                .publish(ToolExecutionCompleted { session_id, result })
                .await;
        });
    }
}

/// Parses arguments and executes the fetch.
async fn execute_fetch(
    web_fetcher: &Arc<dyn WebFetcher>,
    tool_call: &ToolCall,
    session_id: &crate::protocol::SessionId,
    dispatched_at: jiff::Timestamp,
    bus: &crate::common::services::bus_service::BusService,
) -> ToolResult {
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

    // Lazy streaming: same pattern as web-search — the pending ToolResult
    // entry only appears when the fetcher reports a wait.
    let started = std::sync::atomic::AtomicBool::new(false);
    let on_progress: jinn_web_fetch::ProgressFn = std::sync::Arc::new({
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
            }));
        }
    });

    match web_fetcher
        .fetch_observed(&args.url, options, on_progress)
        .await
    {
        Ok(output) => {
            tracing::debug!(
                status = output.status,
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

/// Returns the tool definition for `web-fetch`.
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

fn web_fetch_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "web-fetch".to_owned(),
        description: "Fetch a web page and return its content. By default returns \n            boilerplate-stripped markdown (markdown-clean); request `html` for the raw \n            page source (e.g. when building or inspecting markup), or `markdown` for the \n            full page including nav/footer."
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
                            "enum": ["html", "markdown", "markdown-clean"],
                            "description": "Output format. Defaults to 'markdown-clean' (boilerplate-stripped)."
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

    /// A mock web fetcher that returns a fixed response, optionally after a delay.
    struct MockFetcher {
        content: String,
        success: bool,
        delay: Option<std::time::Duration>,
    }

    #[async_trait]
    impl WebFetcher for MockFetcher {
        async fn fetch(
            &self,
            _url: &str,
            _options: FetchOptions,
        ) -> Result<FetchOutput, jinn_web_fetch::FetchError> {
            if let Some(delay) = self.delay {
                tokio::time::sleep(delay).await;
            }
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
            delay: None,
        })
    }

    fn mock_fetcher_with_error() -> Arc<dyn WebFetcher> {
        Arc::new(MockFetcher {
            content: String::new(),
            success: false,
            delay: None,
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
        let messages = await_recorded(&recorder, 1, std::time::Duration::from_secs(1)).await;
        assert_eq!(messages.len(), 1, "should send exactly one RegisterTools");
        assert_eq!(messages[0].provider, "web-fetch");
        assert_eq!(messages[0].definitions.len(), 1);
        assert_eq!(messages[0].definitions[0].name, "web-fetch");
    }

    #[tokio::test]
    async fn startup_web_fetch_tool_exposes_clean_format_enum() {
        // Given a WebFetchActor.
        let harness = TestHarness::new().await;
        let recorder = harness.spawn_recorder::<RegisterTools>().await;
        let _actor = WebFetchActor::spawn(WebFetchActorDeps {
            deps: harness.actor_deps().await,
            web_fetcher: mock_fetcher_with_success(),
        });

        // When the actor registers its tool.
        let messages = await_recorded(&recorder, 1, std::time::Duration::from_secs(1)).await;

        // Then the format enum is html, markdown, markdown-clean.
        let params = &messages[0].definitions[0].parameters;
        let formats = params["properties"]["options"]["properties"]["format"]["enum"]
            .as_array()
            .expect("format enum should be an array");
        let formats: Vec<&str> = formats
            .iter()
            .map(|v| v.as_str().expect("enum value is a string"))
            .collect();
        assert_eq!(formats, vec!["html", "markdown", "markdown-clean"]);
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
                dispatched_at: jiff::Timestamp::now(),
            })
            .await;

        // Then a ToolExecutionCompleted event was emitted with success.
        let messages = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
        assert_eq!(messages.len(), 1);
        assert!(messages[0].result.success);
        assert_eq!(messages[0].result.content, "Hello, World!");
        assert_eq!(messages[0].session_id, session_id);
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
                dispatched_at: jiff::Timestamp::now(),
            })
            .await;

        // Then a ToolExecutionCompleted event was emitted with failure.
        let messages = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
        assert_eq!(messages.len(), 1);
        assert!(!messages[0].result.success);
        assert!(messages[0].result.content.contains("fetch failed"));
    }

    #[tokio::test]
    async fn execute_web_fetch_concurrent_requests_overlap() {
        // Given a WebFetchActor whose fetcher sleeps 200ms per fetch.
        let harness = TestHarness::new().await;
        let recorder = harness.spawn_recorder::<ToolExecutionCompleted>().await;
        let actor = WebFetchActor::spawn(WebFetchActorDeps {
            deps: harness.actor_deps().await,
            web_fetcher: Arc::new(MockFetcher {
                content: "Hello, World!".to_owned(),
                success: true,
                delay: Some(std::time::Duration::from_millis(200)),
            }),
        });
        actor.wait_for_startup().await;

        // When publishing two ExecuteWebFetch commands back-to-back.
        let start = std::time::Instant::now();
        for id in ["tc_a", "tc_b"] {
            harness
                .publish(ExecuteWebFetch {
                    session_id: SessionId::new(),
                    tool_call: ToolCall {
                        id: id.to_owned(),
                        name: "web-fetch".to_owned(),
                        arguments: r#"{"url": "https://example.com/"}"#.to_owned(),
                    },
                    dispatched_at: jiff::Timestamp::now(),
                })
                .await;
        }

        // Then both completions arrive, and the total wall time reflects
        // overlapping fetches (well under the 400ms a serial mailbox would take).
        let messages = await_recorded(&recorder, 2, std::time::Duration::from_secs(2)).await;
        assert_eq!(messages.len(), 2, "both fetches should complete");
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_millis(380),
            "fetches overlapped (took {elapsed:?}); a serial mailbox would take ~400ms"
        );
    }
}

#[cfg(test)]
mod config_tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use tempfile::TempDir;

    use super::{WebFetchBackend, WebFetchConfig};
    use crate::common::app_info::PREFS_FILE_NAME;
    use crate::feat::preferences_actor::user_preferences::{
        UserPreferences, load_preferences_from, save_preferences_to,
    };

    #[rstest::rstest]
    fn default_web_fetch_config_uses_headless_chrome_backend() {
        let config = WebFetchConfig::default();
        assert_eq!(config.backend, WebFetchBackend::HeadlessChrome);
    }

    #[rstest::rstest]
    fn load_parses_web_fetch_headless_chrome() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"[web_fetch]
backend = "headless-chrome"
"#,
        )
        .expect("write");

        let prefs = load_preferences_from(&path).expect("load");
        assert_eq!(prefs.web_fetch.backend, WebFetchBackend::HeadlessChrome);
    }

    #[rstest::rstest]
    fn load_rejects_invalid_web_fetch_backend() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"[web_fetch]
backend = "socks"
"#,
        )
        .expect("write");

        let result = load_preferences_from(&path);
        assert!(result.is_err());
    }

    #[rstest::rstest]
    fn save_then_load_round_trips_web_fetch_config() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            web_fetch: WebFetchConfig {
                backend: WebFetchBackend::HeadlessChrome,
            },
            ..UserPreferences::default()
        };

        save_preferences_to(&prefs, &path).expect("save");
        let reloaded = load_preferences_from(&path).expect("load");
        assert_eq!(reloaded.web_fetch.backend, WebFetchBackend::HeadlessChrome);
    }

    #[rstest::rstest]
    fn browser_config_defaults_when_absent() {
        // Given a jinn.toml with [web_fetch] but no [browser] table.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(
            &path,
            r#"[web_fetch]
backend = "headless-chrome"
"#,
        )
        .expect("write");

        // When loading.
        let prefs = load_preferences_from(&path).expect("load");

        // Then browser defaults are applied.
        let browser = &prefs.browser;
        assert_eq!(browser.binary, crate::feat::browser::BrowserBinary::Auto);
        assert_eq!(browser.anubis_timeout_secs, 30);
        assert!(browser.user_agent.is_none());
    }

    #[rstest::rstest]
    fn browser_config_round_trips_through_toml() {
        // Given a configured [browser] section.
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        let prefs = UserPreferences {
            browser: crate::feat::browser::BrowserConfig {
                binary: crate::feat::browser::BrowserBinary::Chrome,
                user_agent: Some("Custom/1.0".to_owned()),
                anubis_timeout_secs: 45,
                challenge_wait_secs: 120,
                settle_secs: 5,
                keep_tabs_open: false,
            },
            ..UserPreferences::default()
        };

        // When saving then reloading.
        save_preferences_to(&prefs, &path).expect("save");
        let reloaded = load_preferences_from(&path).expect("load");

        // Then every browser field is preserved.
        let browser = &reloaded.browser;
        assert_eq!(browser.user_agent.as_deref(), Some("Custom/1.0"));
        assert_eq!(browser.binary, crate::feat::browser::BrowserBinary::Chrome);
        assert_eq!(browser.anubis_timeout_secs, 45);
    }

    #[rstest::rstest]
    fn save_preserves_user_comments_in_browser_section() {
        // Given a jinn.toml with comments inside [browser].
        let original = "# my browser notes\n[browser]\n# use real chrome\nbinary = \"chrome\"\n";
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join(PREFS_FILE_NAME);
        std::fs::write(&path, original).expect("write");

        // When loading, changing anubis_timeout_secs, and saving.
        let mut prefs = load_preferences_from(&path).expect("load");
        prefs.browser.anubis_timeout_secs = 60;
        save_preferences_to(&prefs, &path).expect("save");

        // Then the user comments survive the load→patch→save cycle.
        let written = std::fs::read_to_string(&path).expect("read");
        assert!(written.contains("# my browser notes"));
        assert!(written.contains("# use real chrome"));
        assert!(written.contains("anubis_timeout_secs = 60"));
    }
}
