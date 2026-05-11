//! Cucumber `World` wrapping real actors for actor-level integration testing.
//!
//! The [`ActorWorld`] creates an [`AppCore`] with [`InMemoryActorHost`] hosting
//! the real LLM and tool orchestrator actors, backed by a fake LLM factory
//! that simulates multi-turn tool loop behavior.
//!
//! Phase 7: The bus has been deleted. AppCore now uses an async forwarding
//! task to drain the AppMsg channel and forward directly to the actor host.

use std::sync::Arc;

use cucumber::World;
use nullslop_domain::AppState;
use nullslop_domain::Services;
use nullslop_domain::llm::LlmActor;
use nullslop_domain::protocol::provider::SendToLlmProvider;
use nullslop_domain::protocol::tool::ToolCall;
use nullslop_domain::session::actor::{SessionPersistenceActor, SessionPersistenceDirectMsg};
use nullslop_domain::tools::ToolOrchestratorActor;
use nullslop_domain::{Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink};
use nullslop_domain::{ActorHostService, InMemoryActorHost, spawn_actor};
use nullslop_domain::{ActorMessageSink, AppCore, AppMsg};
use nullslop_domain::{FakeLlmServiceFactory, LlmServiceFactoryService, TOOL_LOOP_TRIGGER};

/// Cucumber world wrapping real actors for integration testing.
///
/// Created fresh for each scenario. The LLM actor and tool orchestrator
/// actor are running in-memory, communicating through the async forwarding task.
#[derive(World)]
#[world(init = Self::new_actor_world)]
pub struct ActorWorld {
    /// The application core (state, message channel).
    pub core: AppCore,
    /// Runtime services.
    #[allow(dead_code)]
    pub services: Services,
    /// Actor host for coordinated shutdown.
    #[allow(dead_code)]
    actor_host: ActorHostService,
    /// Tokio runtime handle for spawning async shutdown task.
    #[allow(dead_code)]
    handle: tokio::runtime::Handle,
    /// Receiver for core lifecycle notifications (shutdown complete).
    #[allow(dead_code)]
    core_receiver: kanal::Receiver<nullslop_domain::CoreNotification>,
}

impl std::fmt::Debug for ActorWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActorWorld")
            .field("state", &self.core.state)
            .finish_non_exhaustive()
    }
}

impl ActorWorld {
    /// Creates a new world with real actors backed by the tool loop fake factory.
    fn new_actor_world() -> Self {
        let rt = Box::leak(Box::new(
            tokio::runtime::Runtime::new().expect("test runtime"),
        ));
        let handle = rt.handle().clone();

        let tool_call = ToolCall {
            id: "call_echo_1".to_string(),
            name: "echo".to_string(),
            arguments: r#"{"input":"hello"}"#.to_string(),
        };
        let fake_factory = FakeLlmServiceFactory::with_tool_loop(
            vec!["Let me check".to_string()],
            vec![tool_call],
            vec!["The answer is done".to_string()],
        );
        let llm_service = LlmServiceFactoryService::new(Arc::new(fake_factory));

        let (core, services, actor_host, core_receiver) = create_actor_core(&handle, llm_service);
        Self {
            core,
            services,
            actor_host,
            handle,
            core_receiver,
        }
    }

    /// Submits a command to the core's message channel.
    pub fn submit_command(&self, cmd: nullslop_domain::Command) {
        self.core.submit_command(cmd);
    }

    /// Runs graceful coordinated shutdown of the actor system.
    #[allow(dead_code)]
    pub fn graceful_shutdown(&mut self) {
        nullslop_domain::coordinated_shutdown(
            self.actor_host.backend(),
            &self.core.state,
            &self.core_receiver,
            &self.handle,
            nullslop_domain::SHUTDOWN_TIMEOUT,
        );
    }

    /// Returns a read guard to the application state.
    pub fn state(&self) -> nullslop_domain::StateReadGuard<'_> {
        self.core.state.read()
    }
}

/// Creates an `AppCore` with the LLM actor and tool orchestrator actor
/// running via `InMemoryActorHost`.
fn create_actor_core(
    handle: &tokio::runtime::Handle,
    llm_service: LlmServiceFactoryService,
) -> (
    AppCore,
    Services,
    ActorHostService,
    kanal::Receiver<nullslop_domain::CoreNotification>,
) {
    let (sender, receiver) = kanal::unbounded::<AppMsg>();
    let (core_notify_tx, core_notify_rx) = kanal::unbounded::<nullslop_domain::CoreNotification>();
    let sink = Arc::new(ActorMessageSink::new(sender.clone()));

    // Create tool orchestrator actor.
    let (orch_tx, orch_rx) =
        kanal::unbounded::<ActorEnvelope<nullslop_domain::tools::ToolOrchestratorDirectMsg>>();
    let orch_ref = ActorRef::new(orch_tx);
    let mut orch_ctx = ActorContext::new("tool-orchestrator", sink.clone());
    let orch_actor = ToolOrchestratorActor::activate(&mut orch_ctx);
    let orch_result = spawn_actor(
        "tool-orchestrator",
        orch_actor,
        &orch_ref,
        orch_rx,
        orch_ctx,
        handle,
    );

    // Create LLM actor with fake factory.
    let (llm_tx, llm_rx) = kanal::unbounded::<ActorEnvelope<nullslop_domain::llm::LlmDirectMsg>>();
    let llm_ref = ActorRef::new(llm_tx);
    let mut llm_ctx = ActorContext::new("llm-streaming", sink.clone());
    llm_ctx.set_data(llm_service.clone());
    let llm_actor = LlmActor::activate(&mut llm_ctx);
    let llm_result = spawn_actor(
        "llm-streaming",
        llm_actor,
        &llm_ref,
        llm_rx,
        llm_ctx,
        handle,
    );

    // Create session actor to write events into state.
    // State is shared between AppCore and the session actor via Arc clone.
    let state = nullslop_domain::State::new(AppState::default());
    let (sp_tx, sp_rx) = kanal::unbounded::<ActorEnvelope<SessionPersistenceDirectMsg>>();
    let sp_ref = ActorRef::new(sp_tx);
    let mut sp_ctx = ActorContext::new("session-persistence", sink.clone());
    sp_ctx.set_data(state.clone());
    // SessionStoreService is optional — the e2e test only needs streaming event→state writes.
    let sp_actor = SessionPersistenceActor::activate(&mut sp_ctx);
    let sp_result = spawn_actor(
        "session-persistence",
        sp_actor,
        &sp_ref,
        sp_rx,
        sp_ctx,
        handle,
    );

    let host = InMemoryActorHost::from_actors_with_handle(
        vec![orch_result, llm_result, sp_result],
        handle.clone(),
    );
    let host_arc: Arc<dyn nullslop_domain::ActorHost> = Arc::new(host);

    let services = nullslop_domain::services::test_services::TestServices::builder()
        .handle(handle.clone())
        .llm_service(llm_service)
        .build();

    // Spawn the async forwarding task.
    let actor_host_service = nullslop_domain::ActorHostService::new(host_arc);
    nullslop_domain::spawn_forwarding_task(receiver, actor_host_service.clone(), handle);

    let core = AppCore {
        state: state.clone(),
        sender,
    };

    // Emit lifecycle events for the session actor.
    let _ = sink.send_event(nullslop_domain::Event::ActorStarting {
        payload: nullslop_domain::ActorStarting {
            name: "session-persistence".to_string(),
            description: Some("Session lifecycle and persistence".to_string()),
        },
    });
    let _ = sink.send_event(nullslop_domain::Event::ActorStarted {
        payload: nullslop_domain::ActorStarted {
            name: "session-persistence".to_string(),
            description: Some("Session lifecycle and persistence".to_string()),
        },
    });

    // Core notification sender is wired into services via builder.
    let _ = core_notify_tx; // Not used in actor world — the shutdown tracker isn't running.
    (core, services, actor_host_service, core_notify_rx)
}

// ---------------------------------------------------------------------------
// Step definitions
// ---------------------------------------------------------------------------

#[cucumber::given(expr = "a fresh actor world with the tool loop fake")]
fn given_fresh_actor_world(_world: &mut ActorWorld) {}

#[cucumber::when(expr = "I submit SendToLlmProvider with the tool loop trigger")]
async fn when_submit_tool_loop_trigger(world: &mut ActorWorld) {
    let session_id = world.state().session.active_session.clone();
    world.submit_command(nullslop_domain::Command::SendToLlmProvider {
        payload: SendToLlmProvider {
            session_id,
            messages: vec![nullslop_domain::LlmMessage::User {
                content: TOOL_LOOP_TRIGGER.to_string(),
            }],
            provider_id: None,
        },
    });

    // Async poll until the multi-turn tool loop completes.
    // The session starts idle, so we first wait for it to become
    // non-idle (processing started), then wait for it to return
    // to idle (processing finished).
    let state = world.core.state.clone();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if !state.read().active_session().is_idle() {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if state.read().active_session().is_idle() {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[cucumber::then(expr = "the chat history should contain at least {int} entries")]
fn then_chat_history_at_least(world: &mut ActorWorld, min: u64) {
    let count = world.state().active_session().history().len();
    assert!(
        count >= min as usize,
        "expected at least {min} history entries, got {count}"
    );
}

#[cucumber::then(expr = "the session should be idle")]
fn then_session_idle(world: &mut ActorWorld) {
    assert!(
        world.state().active_session().is_idle(),
        "expected session to be idle"
    );
}
