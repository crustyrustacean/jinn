//! Workflow actor — bridges actor bus events to workflow execution.
//!
//! Subscribes to `StartWorkflow`, `CancelWorkflow`, and `StreamCompleted`.
//! Manages the lifecycle of workflow executions by coordinating between
//! the workflow engine and the actor bus.

use std::sync::Arc;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::provider::protocol::event::StreamCompleted;
use crate::feat::workflow::domain_node_context::DomainNodeContext;
use crate::feat::workflow::protocol::command::{CancelWorkflow, StartWorkflow};
use crate::feat::workflow::protocol::event::{WorkflowCompleted, WorkflowStarted};
use crate::feat::workflow::workflow_registry;
use crate::feat::workflow::workflow_state::WorkflowState;
use crate::protocol::{Command, Event};

/// The workflow actor.
///
/// Bridges `StreamCompleted` events back to pending workflow node executions,
/// and handles `StartWorkflow`/`CancelWorkflow` commands.
pub struct WorkflowActor {
    /// Shared domain node context (holds pending oneshot channels).
    ctx: Arc<DomainNodeContext>,
    /// Shared application state.
    state: State,
}

/// Dependencies for [`WorkflowActor`].
pub struct WorkflowActorDeps {
    /// Shared application state.
    pub state: State,
    /// Runtime services.
    pub services: Services,
}

impl Actor for WorkflowActor {
    type Message = NoDirectMsg;
    type Deps = WorkflowActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<StartWorkflow>();
        ctx.subscribe_command::<CancelWorkflow>();
        ctx.subscribe_event::<StreamCompleted>();

        ctx.set_description("Manages workflow execution lifecycle");

        let domain_ctx = Arc::new(DomainNodeContext::new(deps.services, deps.state.clone()));

        Self {
            ctx: domain_ctx,
            state: deps.state,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(cmd) => self.handle_command(&cmd, ctx),
            ActorEnvelope::Event(Event::StreamCompleted(ref payload)) => {
                self.handle_stream_completed(payload);
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
            // Commands NOT subscribed to — these should not arrive.
            _ => {}
        }
    }

    /// Handle a `StartWorkflow` command.
    fn handle_start_workflow(&mut self, payload: &StartWorkflow, ctx: &ActorContext) {
        let name = &payload.name;
        let workflow_id = payload.workflow_id.clone();

        // Look up the graph builder.
        let Some(builder) = workflow_registry::get_workflow(name) else {
            tracing::warn!(name = %name, "unknown workflow requested");
            return;
        };

        // Build the graph.
        let graph_for_engine = builder(payload.user_prompt.clone());

        // Build a second copy for rendering (WorkflowGraph is not Clone).
        let graph_for_rendering = builder(payload.user_prompt.clone());

        // Create workflow state with the rendering copy.
        let mut workflow_state = WorkflowState::new(name.clone(), graph_for_rendering);
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
                graph_for_engine,
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

    /// Handle a `StreamCompleted` event.
    ///
    /// Correlates the session ID to a pending oneshot channel in
    /// `DomainNodeContext` and resolves it with the response content.
    fn handle_stream_completed(&mut self, payload: &StreamCompleted) {
        let response = payload.assistant_content.clone().unwrap_or_default();
        self.ctx.resolve_completed(&payload.session_id, response);
    }
}

use std::collections::HashMap;
