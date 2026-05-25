//! Workflow actor — bridges actor bus events to workflow execution.
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
use crate::feat::workflow::domain_node_context::DomainNodeContext;
use crate::feat::workflow::protocol::command::{
    CancelWorkflow, LoadWorkflowPickerEntries, RerunFromNode, StartWorkflow,
};
use crate::feat::workflow::protocol::event::{WorkflowCompleted, WorkflowStarted};
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
            // Commands NOT subscribed to — these should not arrive.
            _ => {}
        }
    }

    /// Handle a `StartWorkflow` command.
    fn handle_start_workflow(&mut self, payload: &StartWorkflow, ctx: &ActorContext) {
        let name = &payload.name;
        let workflow_id = payload.workflow_id.clone();

        // Look up the graph builder from the injected registry.
        let Some(builder) = self.registry.get(name) else {
            tracing::warn!(name = %name, "unknown workflow requested");
            return;
        };

        // Build the graph once and wrap in a WorkflowExecution.
        let execution = Arc::new(nullslop_workflow::execution::WorkflowExecution::new(builder()));

        // Create workflow state with the shared execution.
        let mut workflow_state = WorkflowState::new(name.clone(), execution.clone());
        workflow_state.id = workflow_id.clone();

        // Insert into app state.
        self.state.write().workflow.insert(workflow_state);

        // Emit WorkflowStarted event.
        let _ = ctx.send_event(Event::WorkflowStarted(WorkflowStarted {
            workflow_id: workflow_id.clone(),
            name: name.clone(),
        }));

        // Spawn the engine execution as a background task.
        let domain_ctx = self.ctx.clone();
        let state = self.state.clone();
        let cancel = {
            let guard = state.read();
            guard.workflow.get(&workflow_id).map(|w| w.cancel.clone())
        };

        let Some(cancel) = cancel else {
            tracing::warn!(id = %workflow_id, "workflow not found after insert");
            return;
        };

        let ctx_sink = ctx.sink().clone();

        tokio::spawn(async move {
            let result = nullslop_workflow::engine::execute_with_cancel(
                execution,
                domain_ctx.clone(),
                cancel,
            )
            .await;

            match result {
                Ok(workflow_result) => {
                    tracing::info!(id = %workflow_id, "workflow completed successfully");

                    // Update workflow state with result.
                    if let Some(guard) = state.write().workflow.get_mut(&workflow_id) {
                        guard.result =
                            Some(crate::feat::workflow::workflow_state::WorkflowResult {
                                outputs: workflow_result.outputs,
                                success: true,
                            });
                    }

                    // Emit WorkflowCompleted event.
                    let event = Event::WorkflowCompleted(WorkflowCompleted {
                        workflow_id: workflow_id.clone(),
                        success: true,
                    });
                    let _ = ctx_sink.send_event(event);
                }
                Err(report) => {
                    tracing::error!(id = %workflow_id, error = %report, "workflow failed");

                    // Update workflow state with failure.
                    if let Some(guard) = state.write().workflow.get_mut(&workflow_id) {
                        guard.result =
                            Some(crate::feat::workflow::workflow_state::WorkflowResult {
                                outputs: HashMap::new(),
                                success: false,
                            });
                    }

                    // Emit WorkflowCompleted event (failure).
                    let event = Event::WorkflowCompleted(WorkflowCompleted {
                        workflow_id: workflow_id.clone(),
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
        use crate::feat::session::chat_session::SessionPhase;

        // Only care about Idle transitions (session finished all work).
        if payload.new_phase != SessionPhase::Idle {
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
            guard
                .workflow
                .get(&workflow_id)
                .map(|w| w.cancel.clone())
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
            let result = nullslop_workflow::engine::run_pending(
                execution,
                domain_ctx.clone(),
                cancel,
            )
            .await;

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

        let entries: Vec<WorkflowPickerEntry> = self
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

        self.state
            .write()
            .frontend
            .workflow_picker
            .set_items(entries);
    }
}
