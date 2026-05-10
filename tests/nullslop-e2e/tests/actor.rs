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
use nullslop_actor::{Actor, ActorContext, ActorEnvelope, ActorRef};
use nullslop_actor_host::{ActorHostService, InMemoryActorHost, spawn_actor};
use nullslop_component::AppState;
use nullslop_core::{ActorMessageSink, AppCore, AppMsg};
use nullslop_llm::LlmActor;
use nullslop_protocol::provider::SendToLlmProvider;
use nullslop_protocol::tool::ToolCall;
use nullslop_providers::{FakeLlmServiceFactory, LlmServiceFactoryService, TOOL_LOOP_TRIGGER};
use nullslop_services::Services;
use nullslop_tool_orchestrator::ToolOrchestratorActor;

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
    actor_host: ActorHostService,
    /// Tokio runtime handle for spawning async shutdown task.
    handle: tokio::runtime::Handle,
    /// Receiver for core lifecycle notifications (shutdown complete).
    core_receiver: kanal::Receiver<nullslop_protocol::CoreNotification>,
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
    pub fn submit_command(&self, cmd: nullslop_protocol::Command) {
        self.core.submit_command(cmd);
    }

    /// Runs graceful coordinated shutdown of the actor system.
    pub fn graceful_shutdown(&mut self) {
        nullslop_core::coordinated_shutdown(
            self.actor_host.backend(),
            &self.core.state,
            self.core_receiver.clone(),
            self.handle.clone(),
            nullslop_core::SHUTDOWN_TIMEOUT,
        );
    }

    /// Returns a read guard to the application state.
    pub fn state(&self) -> nullslop_core::StateReadGuard<'_> {
        self.core.state.read()
    }
}

/// Creates an `AppCore` with the LLM actor and tool orchestrator actor
/// running via `InMemoryActorHost`.
fn create_actor_core(
    handle: &tokio::runtime::Handle,
    llm_service: LlmServiceFactoryService,
) -> (AppCore, Services, ActorHostService, kanal::Receiver<nullslop_protocol::CoreNotification>)
{
    let (sender, receiver) = kanal::unbounded::<AppMsg>();
    let (core_notify_tx, core_notify_rx) =
        kanal::unbounded::<nullslop_protocol::CoreNotification>();
    let sink = Arc::new(ActorMessageSink::new(sender.clone()));

    // Create tool orchestrator actor.
    let (orch_tx, orch_rx) =
        kanal::unbounded::<ActorEnvelope<nullslop_tool_orchestrator::ToolOrchestratorDirectMsg>>();
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
    let (llm_tx, llm_rx) = kanal::unbounded::<ActorEnvelope<nullslop_llm::LlmDirectMsg>>();
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

    let host =
        InMemoryActorHost::from_actors_with_handle(vec![orch_result, llm_result], handle.clone());
    let host_arc: Arc<dyn nullslop_actor_host::ActorHost> = Arc::new(host);

    let services = nullslop_services::test_services::TestServices::builder()
        .handle(handle.clone())
        .llm_service(llm_service)
        .build();

    // Spawn the async forwarding task.
    let actor_host_service = nullslop_actor_host::ActorHostService::new(host_arc);
    nullslop_core::spawn_forwarding_task(receiver, actor_host_service.clone(), handle);

    let core = AppCore {
        state: nullslop_core::State::new(AppState::default()),
        sender,
    };

    let mut registry = nullslop_component::AppUiRegistry::new();
    nullslop_component::register_all(&mut registry);

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
fn when_submit_tool_loop_trigger(world: &mut ActorWorld) {
    let session_id = world.state().active_session.clone();
    world.submit_command(nullslop_protocol::Command::SendToLlmProvider {
        payload: SendToLlmProvider {
            session_id,
            messages: vec![nullslop_protocol::LlmMessage::User {
                content: TOOL_LOOP_TRIGGER.to_string(),
            }],
            provider_id: None,
        },
    });
    world.graceful_shutdown();
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
