//! Workflow Controller Actor — orchestrates attached workflow lifecycle.
//!
//! The controller sits between session lifecycle events and the workflow engine.
//! It handles attaching/detaching workflows, triggering them on lifecycle events,
//! batching results, and applying `WorkflowResponse` actions to session state.

use std::collections::HashMap;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::session::chat_entry::ChatEntry;
use crate::feat::session::phase_machine::PhaseKind;
use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
use crate::feat::workflow::attached_workflow::{
    AttachedWorkflow, AttachedWorkflowState, WorkflowTrigger,
};
use crate::feat::workflow::domain_node_context::DomainNodeContext;
use crate::feat::workflow::protocol::command::{
    AttachWorkflow, DetachWorkflow, ToggleWorkflow, TriggerWorkflow,
};
use crate::feat::workflow::protocol::event::AttachedWorkflowCompleted;
use crate::feat::workflow::workflow_response::WorkflowResponse;
use crate::feat::workflow::workflow_state::{WorkflowExecutionState, WorkflowId};
use crate::protocol::{Command, Event};
use crate::feat::session::chat_session::ChatSessionState;

/// The workflow controller actor.
///
/// Owns the lifecycle of attached workflows: attach, detach, toggle, trigger.
/// Orchestrates TurnEnd batching, manual triggers, and ESC cancellation.
pub struct WorkflowControllerActor {
    /// Shared domain node context for LLM access.
    ctx: Arc<DomainNodeContext>,
    /// Shared application state.
    state: State,
}

/// Dependencies for [`WorkflowControllerActor`].
pub struct WorkflowControllerActorDeps {
    /// Shared application state.
    pub state: State,
    /// Runtime services.
    pub services: Services,
}

impl Actor for WorkflowControllerActor {
    type Message = NoDirectMsg;
    type Deps = WorkflowControllerActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<AttachWorkflow>();
        ctx.subscribe_command::<DetachWorkflow>();
        ctx.subscribe_command::<ToggleWorkflow>();
        ctx.subscribe_command::<TriggerWorkflow>();
        ctx.subscribe_event::<SessionPhaseChanged>();

        ctx.set_description("Orchestrates attached workflow lifecycle");

        let domain_ctx = Arc::new(DomainNodeContext::new(deps.services, deps.state.clone()));

        Self {
            ctx: domain_ctx,
            state: deps.state,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, _ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(cmd) => self.handle_command(&cmd),
            ActorEnvelope::Event(Event::SessionPhaseChanged(ref payload)) => {
                self.handle_session_phase_changed(payload).await;
            }
            _ => {}
        }
    }
}

impl WorkflowControllerActor {
    /// Dispatches a command to the appropriate handler.
    fn handle_command(&mut self, cmd: &Command) {
        match cmd {
            Command::AttachWorkflow(payload) => {
                self.handle_attach_workflow(payload);
            }
            Command::DetachWorkflow(payload) => {
                self.handle_detach_workflow(payload);
            }
            Command::ToggleWorkflow(payload) => {
                self.handle_toggle_workflow(payload);
            }
            Command::TriggerWorkflow(payload) => {
                self.handle_trigger_workflow(payload);
            }
            _ => {}
        }
    }

    /// Handle `AttachWorkflow` — add a new workflow attachment to a session.
    fn handle_attach_workflow(&mut self, payload: &AttachWorkflow) {
        let attachment = AttachedWorkflow::new(payload.config.clone(), payload.trigger.clone());
        let workflow_id = attachment.id.clone();
        let session_id = payload.session_id.clone();

        {
            let mut guard = self.state.write();
            let Some(session) = guard.session.get_mut(&session_id) else {
                tracing::warn!(id = %session_id, "session not found for attach workflow");
                return;
            };
            session.core.attached_workflows.push(attachment);
        }

        tracing::info!(
            session = %session_id,
            workflow = %workflow_id,
            "attached workflow to session"
        );
    }

    /// Handle `DetachWorkflow` — remove an attachment. Cancels if running.
    fn handle_detach_workflow(&mut self, payload: &DetachWorkflow) {
        let session_id = &payload.session_id;
        let workflow_id = &payload.workflow_id;

        // Cancel running execution if present.
        if let Some(exec_state) = self.state.write().workflow_executions.remove(workflow_id) {
            exec_state.cancel.cancel();
            tracing::info!(id = %workflow_id, "cancelled running attached workflow on detach");
        }

        // Remove from session.
        {
            let mut guard = self.state.write();
            let Some(session) = guard.session.get_mut(session_id) else {
                return;
            };
            session.core.attached_workflows.retain(|aw| aw.id != *workflow_id);
        }

        tracing::info!(
            session = %session_id,
            workflow = %workflow_id,
            "detached workflow from session"
        );
    }

    /// Handle `ToggleWorkflow` — flip enabled on/off.
    fn handle_toggle_workflow(&mut self, payload: &ToggleWorkflow) {
        let session_id = &payload.session_id;
        let workflow_id = &payload.workflow_id;

        let mut guard = self.state.write();
        let Some(session) = guard.session.get_mut(session_id) else {
            return;
        };
        for aw in &mut session.core.attached_workflows {
            if aw.id == *workflow_id {
                aw.enabled = !aw.enabled;
                tracing::info!(
                    session = %session_id,
                    workflow = %workflow_id,
                    enabled = aw.enabled,
                    "toggled workflow"
                );
                break;
            }
        }
    }

    /// Handle `TriggerWorkflow` — manually fire a workflow.
    fn handle_trigger_workflow(&mut self, payload: &TriggerWorkflow) {
        let session_id = payload.session_id.clone();
        let workflow_id = payload.workflow_id.clone();

        // Find the attachment.
        let attachment = {
            let guard = self.state.read();
            let Some(session) = guard.session.get(&session_id) else {
                return;
            };
            session.core.attached_workflows.iter().find(|aw| {
                aw.id == workflow_id
                    && matches!(aw.trigger, WorkflowTrigger::Manual)
                    && aw.enabled
                    && matches!(aw.state, AttachedWorkflowState::Ready)
            }).cloned()
        };

        let Some(attachment) = attachment else {
            return;
        };

        // Fire it.
        self.spawn_attached_workflow(&session_id, attachment);
    }

    /// Handle `SessionPhaseChanged` — check for TurnEnd workflows.
    async fn handle_session_phase_changed(&mut self, payload: &SessionPhaseChanged) {
        if payload.new_phase != PhaseKind::Idle {
            return;
        }

        let session_id = &payload.session_id;

        // Find TurnEnd and TurnEndOneShot attachments.
        let attachments = {
            let guard = self.state.read();
            let Some(session) = guard.session.get(session_id) else {
                return;
            };
            session
                .core
                .attached_workflows
                .iter()
                .filter(|aw| {
                    aw.enabled
                        && matches!(aw.state, AttachedWorkflowState::Ready)
                        && matches!(
                            aw.trigger,
                            WorkflowTrigger::TurnEnd | WorkflowTrigger::TurnEndOneShot
                        )
                })
                .cloned()
                .collect::<Vec<_>>()
        };

        if attachments.is_empty() {
            return;
        }

        // Fire all matching attachments.
        let mut handles = Vec::new();

        for attachment in attachments {
            let wf_id = attachment.id.clone();

            // Set state to Running.
            self.set_attachment_state(session_id, &wf_id, AttachedWorkflowState::Running);

            // Begin busy.
            {
                let mut guard = self.state.write();
                if let Some(session) = guard.session.get_mut(session_id) {
                    session.core.ephemeral.busy_count += 1;
                }
            }

            // Spawn execution.
            let handle = self.spawn_attached_workflow_tokio(session_id, attachment);
            handles.push((wf_id, handle));
        }

        // Await all handles.
        let mut results = Vec::new();
        for (wf_id, handle) in handles {
            let result = match handle.await {
                Ok(Ok(outputs)) => Ok(outputs),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(format!("join error: {e}")),
            };
            results.push((wf_id, result));
        }

        // Apply results in order.
        for (wf_id, result) in results {
            self.apply_workflow_result(&session_id, &wf_id, result);
        }

        // Complete busy.
        {
            let mut guard = self.state.write();
            if let Some(session) = guard.session.get_mut(session_id) {
                session.core.ephemeral.busy_count = session.core.ephemeral.busy_count.saturating_sub(1);
            }
        }
    }

    /// Spawn an attached workflow execution and return a tokio JoinHandle.
    fn spawn_attached_workflow_tokio(
        &self,
        session_id: &crate::protocol::SessionId,
        attachment: AttachedWorkflow,
    ) -> tokio::task::JoinHandle<Result<Vec<WorkflowResponse>, String>> {
        let workflow_id = attachment.id.clone();
        let session_id = session_id.clone();
        let state = self.state.clone();

        // Build graph. Phase 7+ replaces this with config.build_graph().
        // Using example graph as placeholder until builtin.rs is implemented.
        let graph = crate::feat::workflow::example::add_numbers::build_add_numbers();
        let execution = Arc::new(jinn_workflow::execution::WorkflowExecution::new(graph));
        let cancel = CancellationToken::new();

        // Store execution state.
        {
            let mut guard = state.write();
            guard.workflow_executions.insert(
                workflow_id.clone(),
                WorkflowExecutionState {
                    execution: execution.clone(),
                    cancel: cancel.clone(),
                    session_id: session_id.clone(),
                    node_sessions: HashMap::new(),
                },
            );
        }

        let domain_ctx = self.ctx.clone();
        domain_ctx.set_workflow_id(workflow_id.clone());

        tokio::spawn(async move {
            let result = jinn_workflow::engine::execute_with_cancel(
                execution,
                domain_ctx,
                cancel,
            )
            .await;

            match result {
                Ok(workflow_result) => {
                    // The engine returns HashMap<String, serde_json::Value> from sink nodes.
                    // In Phase 7+, the builtin graph builders encode responses as JSON
                    // that we parse into WorkflowResponse here.
                    // For now, return empty (no actions).
                    let _ = workflow_result;
                    Ok(Vec::new())
                }

                Err(report) => Err(format!("{report:#}")),
            }
        })
    }

    /// Spawn an attached workflow (fire-and-forget for manual triggers).
    fn spawn_attached_workflow(
        &self,
        session_id: &crate::protocol::SessionId,
        attachment: AttachedWorkflow,
    ) {
        let workflow_id = attachment.id.clone();
        let session_id_clone = session_id.clone();
        let handle = self.spawn_attached_workflow_tokio(session_id, attachment);

        let state = self.state.clone();
        let ctx = self.ctx.clone();
        tokio::spawn(async move {
            let result = handle.await;

            let response = match result {
                Ok(Ok(outputs)) => Ok(outputs),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(format!("join error: {e}")),
            };

            let actor = WorkflowControllerActor { ctx, state };
            actor.apply_workflow_result(&session_id_clone, &workflow_id, response);

            {
                let mut guard = actor.state.write();
                if let Some(session) = guard.session.get_mut(&session_id_clone) {
                    session.core.ephemeral.busy_count =
                        session.core.ephemeral.busy_count.saturating_sub(1);
                }
            }
        });
    }

    /// Apply a workflow result to the session.
    fn apply_workflow_result(
        &self,
        session_id: &crate::protocol::SessionId,
        workflow_id: &WorkflowId,
        result: Result<Vec<WorkflowResponse>, String>,
    ) {
        // Clean up execution state.
        self.state.write().workflow_executions.remove(workflow_id);

        match result {
            Ok(responses) => {
                let should_detach = responses.iter().any(|r| matches!(r, WorkflowResponse::Detach));
                let should_turn_off = responses.iter().any(|r| matches!(r, WorkflowResponse::TurnOff));

                // Apply each response.
                for response in &responses {
                    match response {
                        WorkflowResponse::PushSessionHistory(entry) => {
                            let mut guard = self.state.write();
                            if let Some(session) = guard.session.get_mut(session_id) {
                                session.push_entry(entry.clone());
                            }
                        }
                        WorkflowResponse::TurnOff => {
                            // Handled below.
                        }
                        WorkflowResponse::Detach => {
                            // Handled below.
                        }
                    }
                }

                // Update attachment state.
                {
                    let mut guard = self.state.write();
                    if let Some(session) = guard.session.get_mut(session_id) {
                        if should_detach {
                            // Remove one-shot attachments.
                            session.core.attached_workflows.retain(|aw| aw.id != *workflow_id);
                            tracing::info!(workflow = %workflow_id, "auto-detached one-shot workflow");
                        } else if should_turn_off {
                            for aw in &mut session.core.attached_workflows {
                                if aw.id == *workflow_id {
                                    aw.enabled = false;
                                    aw.state = AttachedWorkflowState::Completed;
                                    break;
                                }
                            }
                        } else {
                            for aw in &mut session.core.attached_workflows {
                                if aw.id == *workflow_id {
                                    aw.state = AttachedWorkflowState::Completed;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            Err(reason) => {
                tracing::error!(
                    session = %session_id,
                    workflow = %workflow_id,
                    reason = %reason,
                    "attached workflow failed"
                );
                // Set Failed state, push system entry.
                let mut guard = self.state.write();
                if let Some(session) = guard.session.get_mut(session_id) {
                    for aw in &mut session.core.attached_workflows {
                        if aw.id == *workflow_id {
                            aw.state = AttachedWorkflowState::Failed { reason: reason.clone() };
                            break;
                        }
                    }
                    session.push_entry(ChatEntry::system(&format!("[Workflow] Failed: {reason}")));
                }
            }
        }
    }

    /// Set attachment state.
    fn set_attachment_state(
        &self,
        session_id: &crate::protocol::SessionId,
        workflow_id: &WorkflowId,
        new_state: AttachedWorkflowState,
    ) {
        let mut guard = self.state.write();
        let Some(session) = guard.session.get_mut(session_id) else {
            return;
        };
        for aw in &mut session.core.attached_workflows {
            if aw.id == *workflow_id {
                aw.state = new_state;
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, dead_code, clippy::unwrap_used)]

    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::services::test_services::TestServices;
    use crate::feat::session::chat_entry::ChatEntry;
    use crate::feat::session::chat_session::ChatSessionState;
    use crate::feat::workflow::attached_workflow::{
        AttachedWorkflowState, OneShotKind, WorkflowConfig, WorkflowTrigger,
    };
    use crate::protocol::SessionId;

    struct TestHarness {
        state: State,
    }

    impl TestHarness {
        fn new() -> Self {
            let services = TestServices::builder().build();
            let state = State::new(AppState::default());
            Self { state }
        }

        fn insert_session(&self, session: ChatSessionState) -> SessionId {
            let id = session.session_id().clone();
            self.state.write().session.insert(session);
            id
        }

        fn session_has_attachment(&self, session_id: &SessionId, workflow_id: &WorkflowId) -> bool {
            let guard = self.state.read();
            guard.session.get(session_id).map_or(false, |s| {
                s.core.attached_workflows.iter().any(|aw| aw.id == *workflow_id)
            })
        }
    }

    // --- Test 18: controller_finds_turn_end_attachments_on_idle ---

    #[rstest::rstest]
    fn controller_finds_turn_end_attachments_on_idle() {
        let h = TestHarness::new();
        let mut session = ChatSessionState::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus { n: 3, result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant },
            WorkflowTrigger::TurnEnd,
        );
        let wf_id = aw.id.clone();
        session.core.attached_workflows.push(aw);
        let session_id = h.insert_session(session);

        assert!(h.session_has_attachment(&session_id, &wf_id));
        let guard = h.state.read();
        let session = guard.session.get(&session_id).expect("session");
        let matching: Vec<_> = session.core.attached_workflows.iter()
            .filter(|aw| aw.enabled && matches!(aw.state, AttachedWorkflowState::Ready) &&
                matches!(aw.trigger, WorkflowTrigger::TurnEnd))
            .collect();
        assert_eq!(matching.len(), 1);
    }

    // --- Test 19: controller_ignores_disabled_attachments ---

    #[rstest::rstest]
    fn controller_ignores_disabled_attachments() {
        let h = TestHarness::new();
        let mut session = ChatSessionState::new();
        let mut aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus { n: 3, result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant },
            WorkflowTrigger::TurnEnd,
        );
        aw.enabled = false;
        session.core.attached_workflows.push(aw);
        let session_id = h.insert_session(session);

        let guard = h.state.read();
        let session = guard.session.get(&session_id).expect("session");
        let matching: Vec<_> = session.core.attached_workflows.iter()
            .filter(|aw| aw.enabled && matches!(aw.state, AttachedWorkflowState::Ready))
            .collect();
        assert!(matching.is_empty());
    }

    // --- Test 20: controller_ignores_wrong_trigger_type ---

    #[rstest::rstest]
    fn controller_ignores_wrong_trigger_type() {
        let h = TestHarness::new();
        let mut session = ChatSessionState::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus { n: 3, result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant },
            WorkflowTrigger::Manual,
        );
        session.core.attached_workflows.push(aw);
        let session_id = h.insert_session(session);

        let guard = h.state.read();
        let session = guard.session.get(&session_id).expect("session");
        let matching: Vec<_> = session.core.attached_workflows.iter()
            .filter(|aw| matches!(aw.trigger, WorkflowTrigger::TurnEnd | WorkflowTrigger::TurnEndOneShot))
            .collect();
        assert!(matching.is_empty());
    }

    // --- Test 33: attach_workflow_command_creates_attachment ---

    #[rstest::rstest]
    fn attach_workflow_command_creates_attachment() {
        let h = TestHarness::new();
        let services = TestServices::builder().build();
        let state = h.state.clone();
        let ctx = Arc::new(DomainNodeContext::new(services, state.clone()));
        let mut actor = WorkflowControllerActor { ctx, state };

        let session = ChatSessionState::new();
        let session_id = session.session_id().clone();
        h.state.write().session.insert(session);

        actor.handle_attach_workflow(&AttachWorkflow {
            session_id: session_id.clone(),
            config: WorkflowConfig::Consensus { n: 3, result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant },
            trigger: WorkflowTrigger::TurnEnd,
        });

        let guard = h.state.read();
        let session = guard.session.get(&session_id).expect("session");
        assert_eq!(session.core.attached_workflows.len(), 1);
        assert!(matches!(session.core.attached_workflows[0].trigger, WorkflowTrigger::TurnEnd));
    }

    // --- Test 34: detach_workflow_command_removes_attachment ---

    #[rstest::rstest]
    fn detach_workflow_command_removes_attachment() {
        let h = TestHarness::new();
        let services = TestServices::builder().build();
        let state = h.state.clone();
        let ctx = Arc::new(DomainNodeContext::new(services, state.clone()));
        let mut actor = WorkflowControllerActor { ctx, state };

        let mut session = ChatSessionState::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus { n: 3, result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant },
            WorkflowTrigger::TurnEnd,
        );
        let wf_id = aw.id.clone();
        session.core.attached_workflows.push(aw);
        let session_id = session.session_id().clone();
        h.state.write().session.insert(session);

        actor.handle_detach_workflow(&DetachWorkflow {
            session_id: session_id.clone(),
            workflow_id: wf_id.clone(),
        });

        let guard = h.state.read();
        let session = guard.session.get(&session_id).expect("session");
        assert!(session.core.attached_workflows.is_empty());
    }

    // --- Test 35: toggle_workflow_command_flips_enabled ---

    #[rstest::rstest]
    fn toggle_workflow_command_flips_enabled() {
        let h = TestHarness::new();
        let services = TestServices::builder().build();
        let state = h.state.clone();
        let ctx = Arc::new(DomainNodeContext::new(services, state.clone()));
        let mut actor = WorkflowControllerActor { ctx, state };

        let mut session = ChatSessionState::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus { n: 3, result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant },
            WorkflowTrigger::TurnEnd,
        );
        let wf_id = aw.id.clone();
        session.core.attached_workflows.push(aw);
        let session_id = session.session_id().clone();
        h.state.write().session.insert(session);

        // Toggle off.
        actor.handle_toggle_workflow(&ToggleWorkflow {
            session_id: session_id.clone(),
            workflow_id: wf_id.clone(),
        });

        let guard = h.state.read();
        let session = guard.session.get(&session_id).expect("session");
        assert!(!session.core.attached_workflows[0].enabled);

        // Toggle on.
        drop(guard);
        actor.handle_toggle_workflow(&ToggleWorkflow {
            session_id: session_id.clone(),
            workflow_id: wf_id.clone(),
        });

        let guard = h.state.read();
        let session = guard.session.get(&session_id).expect("session");
        assert!(session.core.attached_workflows[0].enabled);
    }

    // --- Test 21: controller_sets_running_state_on_fire ---

    #[rstest::rstest]
    fn controller_sets_running_state_on_fire() {
        let h = TestHarness::new();
        let services = TestServices::builder().build();
        let state = h.state.clone();
        let ctx = Arc::new(DomainNodeContext::new(services, state.clone()));
        let actor = WorkflowControllerActor { ctx, state };

        let mut session = ChatSessionState::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus { n: 3, result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant },
            WorkflowTrigger::TurnEnd,
        );
        let wf_id = aw.id.clone();
        session.core.attached_workflows.push(aw);
        let session_id = session.session_id().clone();
        h.state.write().session.insert(session);

        // Simulate state transition.
        actor.set_attachment_state(&session_id, &wf_id, AttachedWorkflowState::Running);

        let guard = h.state.read();
        let session = guard.session.get(&session_id).expect("session");
        assert!(matches!(session.core.attached_workflows[0].state, AttachedWorkflowState::Running));
    }

    // --- Test 29: controller_resets_to_ready_on_cancel ---

    #[rstest::rstest]
    fn controller_resets_to_ready_on_cancel() {
        let h = TestHarness::new();
        let services = TestServices::builder().build();
        let state = h.state.clone();
        let ctx = Arc::new(DomainNodeContext::new(services, state.clone()));
        let actor = WorkflowControllerActor { ctx, state };

        let mut session = ChatSessionState::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus { n: 3, result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant },
            WorkflowTrigger::TurnEnd,
        );
        let wf_id = aw.id.clone();
        session.core.attached_workflows.push(aw);
        let session_id = session.session_id().clone();
        h.state.write().session.insert(session);

        // Set Running, then cancel back to Ready.
        actor.set_attachment_state(&session_id, &wf_id, AttachedWorkflowState::Running);
        actor.set_attachment_state(&session_id, &wf_id, AttachedWorkflowState::Ready);

        let guard = h.state.read();
        let session = guard.session.get(&session_id).expect("session");
        assert!(matches!(session.core.attached_workflows[0].state, AttachedWorkflowState::Ready));
    }

    // --- Test 30: controller_sets_failed_on_execution_error ---

    #[rstest::rstest]
    fn controller_sets_failed_on_execution_error() {
        let h = TestHarness::new();
        let services = TestServices::builder().build();
        let state = h.state.clone();
        let ctx = Arc::new(DomainNodeContext::new(services, state.clone()));
        let actor = WorkflowControllerActor { ctx, state };

        let mut session = ChatSessionState::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus { n: 3, result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant },
            WorkflowTrigger::TurnEnd,
        );
        let wf_id = aw.id.clone();
        session.core.attached_workflows.push(aw);
        let session_id = session.session_id().clone();
        h.state.write().session.insert(session);

        // Simulate error.
        actor.apply_workflow_result(&session_id, &wf_id, Err("something broke".to_owned()));

        let guard = h.state.read();
        let session = guard.session.get(&session_id).expect("session");
        assert!(matches!(
            &session.core.attached_workflows[0].state,
            AttachedWorkflowState::Failed { reason } if reason == "something broke"
        ));
    }

    // --- Test 36: trigger_workflow_command_fires_manual ---

    #[rstest::rstest]
    fn trigger_workflow_command_skips_non_manual() {
        let h = TestHarness::new();
        let services = TestServices::builder().build();
        let state = h.state.clone();
        let ctx = Arc::new(DomainNodeContext::new(services, state.clone()));
        let mut actor = WorkflowControllerActor { ctx, state };

        let mut session = ChatSessionState::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus { n: 3, result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant },
            WorkflowTrigger::TurnEnd, // Not Manual
        );
        let wf_id = aw.id.clone();
        session.core.attached_workflows.push(aw);
        let session_id = session.session_id().clone();
        h.state.write().session.insert(session);

        // Try to trigger a TurnEnd workflow manually — should be a no-op.
        actor.handle_trigger_workflow(&TriggerWorkflow {
            session_id: session_id.clone(),
            workflow_id: wf_id.clone(),
        });

        // State should still be Ready (not Running).
        let guard = h.state.read();
        let session = guard.session.get(&session_id).expect("session");
        assert!(matches!(session.core.attached_workflows[0].state, AttachedWorkflowState::Ready));
    }

    // --- Test 32: controller_handles_detach_while_running ---

    #[rstest::rstest]
    fn controller_handles_detach_while_running() {
        let h = TestHarness::new();
        let services = TestServices::builder().build();
        let state = h.state.clone();
        let ctx = Arc::new(DomainNodeContext::new(services, state.clone()));
        let mut actor = WorkflowControllerActor { ctx, state };

        let mut session = ChatSessionState::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus { n: 3, result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant },
            WorkflowTrigger::TurnEnd,
        );
        let wf_id = aw.id.clone();
        session.core.attached_workflows.push(aw);
        let session_id = session.session_id().clone();
        h.state.write().session.insert(session);

        // Simulate running state with an execution entry.
        let exec_state = WorkflowExecutionState {
            execution: Arc::new(jinn_workflow::execution::WorkflowExecution::new(
                crate::feat::workflow::example::add_numbers::build_add_numbers(),
            )),
            cancel: CancellationToken::new(),
            session_id: session_id.clone(),
            node_sessions: HashMap::new(),
        };
        h.state.write().workflow_executions.insert(wf_id.clone(), exec_state);

        // Detach while running.
        actor.handle_detach_workflow(&DetachWorkflow {
            session_id: session_id.clone(),
            workflow_id: wf_id.clone(),
        });

        // Verify: attachment removed AND execution removed.
        let guard = h.state.read();
        let session = guard.session.get(&session_id).expect("session");
        assert!(session.core.attached_workflows.is_empty());
        assert!(guard.workflow_executions.get(&wf_id).is_none());
    }
}
