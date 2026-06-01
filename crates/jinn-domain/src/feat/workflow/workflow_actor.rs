//! Workflow actor - bridges actor bus events to workflow execution.
//!
//! Subscribes to `StartWorkflow`, `CancelWorkflow`, and `SessionPhaseChanged`.
//! Manages the lifecycle of workflow executions by coordinating between
//! the workflow engine and the actor bus.

use std::collections::HashMap;
use std::sync::Arc;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::session::chat_entry::ChatEntryKind;
use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
use crate::feat::ui::picker_states::PickerExt;
use crate::feat::workflow::domain_node_context::DomainNodeContext;
use crate::feat::workflow::protocol::command::{
    CancelWorkflow, InitWorkflow, LoadWorkflowPickerEntries, RerunFromNode, StartWorkflow,
};
use crate::feat::workflow::protocol::event::{
    WorkflowCompleted, WorkflowInitialized, WorkflowStarted,
};
use crate::feat::workflow::workflow_registry::WorkflowRegistry;
use crate::feat::workflow::workflow_state::WorkflowState;
use crate::protocol::{Command, Event};

/// The workflow actor.
///
/// Bridges `SessionPhaseChanged(Idle)` events back to pending workflow node
/// executions, and handles `StartWorkflow`/`CancelWorkflow` commands.
pub struct WorkflowActor {
    /// Shared domain node context (holds pending oneshot channels).
    ctx: Arc<DomainNodeContext>,
    /// Shared application state.
    state: State,
    /// Workflow registry (injected, not global).
    registry: Arc<WorkflowRegistry>,
}

/// Dependencies for [`WorkflowActor`].
pub struct WorkflowActorDeps {
    /// Shared application state.
    pub state: State,
    /// Runtime services.
    pub services: Services,
    /// Workflow registry (built during startup).
    pub registry: Arc<WorkflowRegistry>,
}

impl Actor for WorkflowActor {
    type Message = NoDirectMsg;
    type Deps = WorkflowActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<InitWorkflow>();
        ctx.subscribe_command::<StartWorkflow>();
        ctx.subscribe_command::<CancelWorkflow>();
        ctx.subscribe_command::<RerunFromNode>();
        ctx.subscribe_command::<LoadWorkflowPickerEntries>();
        ctx.subscribe_event::<SessionPhaseChanged>();

        ctx.set_description("Manages workflow execution lifecycle");

        let domain_ctx = Arc::new(DomainNodeContext::new(deps.services, deps.state.clone()));

        Self {
            ctx: domain_ctx,
            state: deps.state,
            registry: deps.registry,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(cmd) => self.handle_command(&cmd, ctx),
            ActorEnvelope::Event(Event::SessionPhaseChanged(ref payload)) => {
                self.handle_session_phase_changed(payload);
            }
            _ => {}
        }
    }
}

impl WorkflowActor {
    /// Dispatches a command to the appropriate handler.
    fn handle_command(&mut self, cmd: &Command, ctx: &ActorContext) {
        match cmd {
            Command::InitWorkflow(payload) => {
                self.handle_init_workflow(payload, ctx);
            }
            Command::StartWorkflow(payload) => {
                self.handle_start_workflow(payload, ctx);
            }
            Command::CancelWorkflow(payload) => {
                self.handle_cancel_workflow(payload);
            }
            Command::RerunFromNode(payload) => {
                self.handle_rerun_from_node(payload, ctx);
            }
            Command::LoadWorkflowPickerEntries(_) => {
                self.handle_load_workflow_picker_entries();
            }
            // Commands NOT subscribed to - these should not arrive.
            _ => {}
        }
    }

    /// Handle an `InitWorkflow` command.
    ///
    /// Loads the workflow graph and creates the execution state, but does NOT
    /// spawn the engine. The user must press Enter (WorkflowRun) to start execution.
    fn handle_init_workflow(&mut self, payload: &InitWorkflow, ctx: &ActorContext) {
        let name = &payload.name;
        let workflow_id = payload.workflow_id.clone();

        // Look up the graph builder from the injected registry.
        let Some(builder) = self.registry.get(name) else {
            tracing::warn!(name = %name, "unknown workflow requested");
            return;
        };

        // Build the graph once and wrap in a WorkflowExecution.
        let execution = Arc::new(jinn_workflow::execution::WorkflowExecution::new(builder()));

        // Create workflow state with the shared execution.
        let mut workflow_state = WorkflowState::new(name.clone(), execution.clone());
        workflow_state.id = workflow_id.clone();

        // Insert into app state.
        self.state.write().workflow.insert(workflow_state);

        // Mark source nodes as AwaitingInput so the UI shows they need user data.
        {
            let snapshot = execution.snapshot();
            let sources = snapshot.structure().sources();
            for source_name in sources {
                execution.set_status(
                    source_name,
                    jinn_workflow::engine::NodeStatus::AwaitingInput,
                );
            }
        }

        // Emit WorkflowInitialized event.
        let _ = ctx.send_event(Event::WorkflowInitialized(WorkflowInitialized {
            workflow_id: workflow_id.clone(),
            name: name.clone(),
        }));

        // Do NOT spawn execute_with_cancel() here.
        // The user must press Enter (WorkflowRun) to start execution.
    }

    /// Handle a `StartWorkflow` command.
    ///
    /// Runs the engine on an already-loaded workflow (initialized via `InitWorkflow`).
    /// Looks up the `WorkflowExecution` from `WorkflowMap` and spawns execution.
    fn handle_start_workflow(&mut self, payload: &StartWorkflow, ctx: &ActorContext) {
        let workflow_id = &payload.workflow_id;

        // Look up the already-loaded workflow.
        let (execution, cancel) = {
            let guard = self.state.read();
            let Some(workflow) = guard.workflow.get(workflow_id) else {
                tracing::warn!(id = %workflow_id, "workflow not found for start - was it initialized?");
                return;
            };
            (workflow.execution.clone(), workflow.cancel.clone())
        };

        // Emit WorkflowStarted event.
        let _ = ctx.send_event(Event::WorkflowStarted(WorkflowStarted {
            workflow_id: workflow_id.clone(),
            name: payload.name.clone(),
        }));

        // Spawn the engine execution as a background task.
        let domain_ctx = self.ctx.clone();
        let state = self.state.clone();
        let ctx_sink = ctx.sink().clone();
        let workflow_id_clone = workflow_id.clone();

        tokio::spawn(async move {
            let result =
                jinn_workflow::engine::execute_with_cancel(execution, domain_ctx.clone(), cancel)
                    .await;

            match result {
                Ok(workflow_result) => {
                    tracing::info!(id = %workflow_id_clone, "workflow completed successfully");

                    // Update workflow state with result.
                    if let Some(guard) = state.write().workflow.get_mut(&workflow_id_clone) {
                        guard.result =
                            Some(crate::feat::workflow::workflow_state::WorkflowResult {
                                outputs: workflow_result.outputs,
                                success: true,
                            });
                    }

                    // Emit WorkflowCompleted event.
                    let event = Event::WorkflowCompleted(WorkflowCompleted {
                        workflow_id: workflow_id_clone.clone(),
                        success: true,
                    });
                    let _ = ctx_sink.send_event(event);
                }
                Err(report) => {
                    tracing::error!(id = %workflow_id_clone, error = %report, "workflow failed");

                    // Update workflow state with failure.
                    if let Some(guard) = state.write().workflow.get_mut(&workflow_id_clone) {
                        guard.result =
                            Some(crate::feat::workflow::workflow_state::WorkflowResult {
                                outputs: HashMap::new(),
                                success: false,
                            });
                    }

                    // Emit WorkflowCompleted event (failure).
                    let event = Event::WorkflowCompleted(WorkflowCompleted {
                        workflow_id: workflow_id_clone.clone(),
                        success: false,
                    });
                    let _ = ctx_sink.send_event(event);
                }
            }
        });
    }

    /// Handle a `CancelWorkflow` command.
    fn handle_cancel_workflow(&mut self, payload: &CancelWorkflow) {
        let guard = self.state.read();
        if let Some(workflow) = guard.workflow.get(&payload.workflow_id) {
            workflow.cancel.cancel();
            tracing::info!(id = %payload.workflow_id, "workflow cancellation requested");
        }
    }

    /// Handle a `SessionPhaseChanged` event.
    ///
    /// When a workflow session transitions to `Idle`, extracts the last assistant
    /// message from the session history and resolves the pending oneshot channel.
    fn handle_session_phase_changed(&mut self, payload: &SessionPhaseChanged) {
        use crate::feat::session::phase_machine::PhaseKind;

        // Only care about Idle transitions (session finished all work).
        if payload.new_phase != PhaseKind::Idle {
            return;
        }

        // Check if this is a workflow session with a pending oneshot.
        if !self.ctx.has_pending(&payload.session_id) {
            return;
        }

        // Read the last assistant message from session history.
        let response = {
            let guard = self.state.read();
            let Some(session) = guard.session.get(&payload.session_id) else {
                return;
            };

            session
                .history()
                .iter()
                .rev()
                .find_map(|entry| match &entry.kind {
                    ChatEntryKind::Assistant(text) => Some(text.clone()),
                    _ => None,
                })
                .unwrap_or_default()
        };

        self.ctx.resolve_completed(&payload.session_id, response);
    }

    /// Handle a `RerunFromNode` command.
    ///
    /// Spawns `run_pending()` on the existing execution with a fresh
    /// cancellation token. The intent handler has already called
    /// `invalidate_from()` and `seed_inputs()` before sending this command.
    fn handle_rerun_from_node(&mut self, payload: &RerunFromNode, ctx: &ActorContext) {
        let workflow_id = payload.workflow_id.clone();
        let node_name = payload.node_name.clone();

        let execution = {
            let guard = self.state.read();
            guard
                .workflow
                .get(&workflow_id)
                .map(|w| w.execution.clone())
        };
        let Some(execution) = execution else {
            tracing::warn!(id = %workflow_id, "workflow not found for rerun");
            return;
        };

        let cancel = {
            let guard = self.state.read();
            guard.workflow.get(&workflow_id).map(|w| w.cancel.clone())
        };
        let Some(cancel) = cancel else {
            tracing::warn!(id = %workflow_id, "workflow cancel token not found for rerun");
            return;
        };

        let domain_ctx = self.ctx.clone();
        let state = self.state.clone();
        let ctx_sink = ctx.sink().clone();

        tracing::info!(
            id = %workflow_id,
            node = %node_name,
            "re-running workflow from node"
        );

        tokio::spawn(async move {
            let result =
                jinn_workflow::engine::run_pending(execution, domain_ctx.clone(), cancel).await;

            match result {
                Ok(workflow_result) => {
                    tracing::info!(id = %workflow_id, "workflow rerun completed successfully");

                    if let Some(guard) = state.write().workflow.get_mut(&workflow_id) {
                        guard.result =
                            Some(crate::feat::workflow::workflow_state::WorkflowResult {
                                outputs: workflow_result.outputs,
                                success: true,
                            });
                    }

                    let event = Event::WorkflowCompleted(WorkflowCompleted {
                        workflow_id: workflow_id.clone(),
                        success: true,
                    });
                    let _ = ctx_sink.send_event(event);
                }
                Err(report) => {
                    tracing::error!(id = %workflow_id, error = %report, "workflow rerun failed");

                    if let Some(guard) = state.write().workflow.get_mut(&workflow_id) {
                        guard.result =
                            Some(crate::feat::workflow::workflow_state::WorkflowResult {
                                outputs: HashMap::new(),
                                success: false,
                            });
                    }

                    let event = Event::WorkflowCompleted(WorkflowCompleted {
                        workflow_id: workflow_id.clone(),
                        success: false,
                    });
                    let _ = ctx_sink.send_event(event);
                }
            }
        });
    }

    /// Handle a `LoadWorkflowPickerEntries` command.
    ///
    /// Iterates all registered workflow names, builds each graph to read its
    /// description, and populates the workflow picker in app state.
    fn handle_load_workflow_picker_entries(&mut self) {
        use crate::feat::workflow::picker_entry::WorkflowPickerEntry;

        let theme = self.state.read().frontend.theme.clone();

        let mut entries: Vec<WorkflowPickerEntry> = self
            .registry
            .names()
            .into_iter()
            .map(|name| {
                let builder = self
                    .registry
                    .get(&name)
                    .expect("names() returns registered builders");
                let graph = builder();
                WorkflowPickerEntry {
                    name,
                    description: graph.description().map(String::from),
                    theme: theme.clone(),
                }
            })
            .collect();

        entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        self.state
            .write()
            .frontend
            .workflow_picker_mut()
            .set_items(entries);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, dead_code)]

    use super::*;
    use crate::common::actor::message_sink::RecordingSink;
    use crate::common::app_state::AppState;
    use crate::common::services::test_services::TestServices;
    use crate::common::state::State;
    use crate::feat::session::chat_entry::ChatEntry;
    use crate::feat::session::chat_session::ChatSessionState;
    use crate::feat::session::phase_machine::PhaseKind;
    use crate::feat::workflow::WorkflowId;
    use crate::feat::workflow::example::add_numbers;
    use crate::protocol::SessionId;
    use std::sync::Arc;

    fn make_registry() -> Arc<WorkflowRegistry> {
        let mut registry = WorkflowRegistry::new();
        registry.register("add-numbers", add_numbers::build_add_numbers);
        Arc::new(registry)
    }

    struct TestHarness {
        actor: WorkflowActor,
        state: State,
        sink: Arc<RecordingSink>,
        registry: Arc<WorkflowRegistry>,
    }

    impl TestHarness {
        fn new() -> Self {
            let services = TestServices::builder().build();
            let state = State::new(AppState::default());
            let registry = make_registry();
            let sink = Arc::new(RecordingSink::new());
            let ctx = Arc::new(DomainNodeContext::new(services, state.clone()));

            let actor = WorkflowActor {
                ctx,
                state: state.clone(),
                registry: registry.clone(),
            };

            Self {
                actor,
                state,
                sink,
                registry,
            }
        }

        fn context(&self) -> &ActorContext {
            // We don't use the real ActorContext for most tests,
            // but we can construct one for event-sending tests.
            unimplemented!("use actor methods directly")
        }

        fn actor_ctx(&self) -> &ActorContext {
            unimplemented!("use actor methods directly")
        }
    }

    fn make_actor_context(sink: &Arc<RecordingSink>) -> ActorContext {
        ActorContext::new(
            "test-workflow-actor",
            sink.clone() as Arc<dyn crate::common::actor::MessageSink>,
        )
    }

    // --- handle_init_workflow ---

    #[rstest::rstest]
    fn handle_init_workflow_creates_state() {
        // Given a fresh harness.
        let mut h = TestHarness::new();
        let sink = h.sink.clone();
        let ctx = make_actor_context(&sink);
        let workflow_id = WorkflowId::new();

        // When initializing a workflow.
        h.actor.handle_init_workflow(
            &InitWorkflow {
                name: "add-numbers".to_owned(),
                workflow_id: workflow_id.clone(),
            },
            &ctx,
        );

        // Then the workflow is in state.
        let guard = h.state.read();
        let wf = guard.workflow.get(&workflow_id).expect("should exist");
        assert_eq!(wf.name, "add-numbers");
        assert!(wf.result.is_none());
    }

    #[rstest::rstest]
    fn handle_init_workflow_emits_initialized_event() {
        // Given a fresh harness.
        let mut h = TestHarness::new();
        let sink = h.sink.clone();
        let ctx = make_actor_context(&sink);
        let workflow_id = WorkflowId::new();

        // When initializing a workflow.
        h.actor.handle_init_workflow(
            &InitWorkflow {
                name: "add-numbers".to_owned(),
                workflow_id: workflow_id.clone(),
            },
            &ctx,
        );

        // Then a WorkflowInitialized event was emitted.
        let events = h.sink.take_events();
        let init_event = events.iter().find_map(|e| match e {
            Event::WorkflowInitialized(e) => Some(e),
            _ => None,
        });
        assert!(init_event.is_some(), "should emit WorkflowInitialized");
        assert_eq!(init_event.unwrap().name, "add-numbers");
    }

    #[rstest::rstest]
    fn handle_init_workflow_unknown_name_is_noop() {
        // Given a fresh harness.
        let mut h = TestHarness::new();
        let sink = h.sink.clone();
        let ctx = make_actor_context(&sink);
        let workflow_id = WorkflowId::new();

        // When initializing with an unknown name.
        h.actor.handle_init_workflow(
            &InitWorkflow {
                name: "nonexistent".to_owned(),
                workflow_id: workflow_id.clone(),
            },
            &ctx,
        );

        // Then no workflow is in state.
        let guard = h.state.read();
        assert!(guard.workflow.get(&workflow_id).is_none());
    }

    // --- handle_cancel_workflow ---

    #[rstest::rstest]
    fn handle_cancel_workflow_cancels_token() {
        // Given a harness with an initialized workflow.
        let mut h = TestHarness::new();
        let sink = h.sink.clone();
        let ctx = make_actor_context(&sink);
        let workflow_id = WorkflowId::new();
        h.actor.handle_init_workflow(
            &InitWorkflow {
                name: "add-numbers".to_owned(),
                workflow_id: workflow_id.clone(),
            },
            &ctx,
        );

        // When canceling.
        h.actor.handle_cancel_workflow(&CancelWorkflow {
            workflow_id: workflow_id.clone(),
        });

        // Then the cancellation token is cancelled.
        let guard = h.state.read();
        let wf = guard.workflow.get(&workflow_id).expect("should exist");
        assert!(wf.cancel.is_cancelled());
    }

    // --- handle_session_phase_changed ---

    #[rstest::rstest]
    fn handle_session_phase_ignores_non_idle() {
        // Given a harness with a pending session.
        let mut h = TestHarness::new();
        let session_id = SessionId::new();

        // Insert a fake pending entry.
        let (tx, _rx) = tokio::sync::oneshot::channel();
        h.actor.ctx.insert_pending(session_id.clone(), tx);

        // When receiving a Streaming phase change.
        h.actor.handle_session_phase_changed(&SessionPhaseChanged {
            session_id: session_id.clone(),
            old_phase: PhaseKind::Idle,
            new_phase: PhaseKind::Streaming,
        });

        // Then the pending entry is still there (not resolved).
        assert!(h.actor.ctx.has_pending(&session_id));
    }

    #[rstest::rstest]
    fn handle_session_phase_resolves_idle_with_response() {
        // Given a harness with a workflow session that has a pending oneshot.
        let mut h = TestHarness::new();

        // Create a session with an assistant message.
        let mut session = ChatSessionState::new();
        session.core.is_workflow = true;
        session.push_entry(ChatEntry::assistant("expected response"));
        let session_id = session.session_id().clone();

        {
            let mut state = h.state.write();
            state.session.insert(session);
            state.session.set_active(session_id.clone());
        }

        // Insert a pending oneshot.
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        h.actor.ctx.insert_pending(session_id.clone(), tx);

        // When receiving an Idle phase change.
        h.actor.handle_session_phase_changed(&SessionPhaseChanged {
            session_id: session_id.clone(),
            old_phase: PhaseKind::Streaming,
            new_phase: PhaseKind::Idle,
        });

        // Then the oneshot is resolved with the assistant message.
        let response = rx.try_recv().expect("should have response");
        assert_eq!(response, "expected response");
    }

    #[rstest::rstest]
    fn handle_session_phase_ignores_idle_without_pending() {
        // Given a harness with a session but no pending oneshot.
        let mut h = TestHarness::new();
        let session_id = SessionId::new();

        let mut session = ChatSessionState::new();
        session.core.is_workflow = true;
        {
            let mut state = h.state.write();
            state.session.insert(session);
            state.session.set_active(session_id.clone());
        }

        // When receiving an Idle phase change (no pending oneshot).
        h.actor.handle_session_phase_changed(&SessionPhaseChanged {
            session_id: session_id.clone(),
            old_phase: PhaseKind::Streaming,
            new_phase: PhaseKind::Idle,
        });

        // Then nothing panicked and no response was sent.
    }

    // --- handle_load_workflow_picker_entries ---

    #[rstest::rstest]
    fn handle_load_workflow_picker_entries_populates_picker() {
        // Given a harness with a registered workflow.
        let mut h = TestHarness::new();

        // When loading picker entries.
        h.actor.handle_load_workflow_picker_entries();

        // Then the picker has entries.
        let guard = h.state.read();
        let items = guard.frontend.workflow_picker().items();
        assert!(!items.is_empty());
        assert_eq!(items[0].name, "add-numbers");
    }

    // --- handle_rerun_from_node ---

    #[rstest::rstest]
    fn handle_rerun_from_node_unknown_workflow_is_noop() {
        // Given a harness without the target workflow.
        let mut h = TestHarness::new();
        let sink = h.sink.clone();
        let ctx = make_actor_context(&sink);

        // When rerunning from a node on a nonexistent workflow.
        h.actor.handle_rerun_from_node(
            &RerunFromNode {
                workflow_id: WorkflowId::new(),
                node_name: "source".to_owned(),
            },
            &ctx,
        );

        // Then no events were emitted.
        let events = h.sink.take_events();
        let completed: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, Event::WorkflowCompleted(_)))
            .collect();
        assert!(completed.is_empty());
    }
}
