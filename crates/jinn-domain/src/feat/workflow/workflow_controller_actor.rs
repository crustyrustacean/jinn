//! Workflow Controller Actor — orchestrates attached workflow lifecycle.
//!
//! Spawning Lua workflow executions and applying results to session state.

use std::collections::HashMap;
use std::sync::Arc;



use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind};

use crate::feat::session::phase_machine::PhaseKind;

use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;

use crate::feat::workflow::attached_workflow::{
    AttachedWorkflow, AttachedWorkflowState, BeforeTurnMode, PromptMergeStrategy, WorkflowConfig,
    WorkflowId, WorkflowTrigger,
};

use crate::feat::workflow::domain_node_context::DomainNodeContext;
use crate::feat::workflow::protocol::command::{
    AttachWorkflow, DetachWorkflow, FireBeforeTurn, ToggleWorkflow, TriggerWorkflow,
};


use crate::feat::chat_input::protocol::command::{EnqueueUserMessage, SetChatInputText};

use crate::protocol::{Command, Event};

use serde::Serialize;

/// The workflow controller actor.
///
/// Owns the lifecycle of attached workflows: attach, detach, toggle, trigger.
/// Orchestrates TurnEnd batching, manual triggers, and ESC cancellation.
///
/// Ephemeral workflow state (`workflow_executions`, `pending_before_turn`,
/// `before_turn_queue`) lives in actor fields — no locking needed since
/// the actor processes one message at a time.
pub struct WorkflowControllerActor {
    /// Shared domain node context for LLM access.
    ctx: Arc<DomainNodeContext>,
    /// Shared application state.
    state: State,
    /// Runtime services.
    services: Services,
    /// Plugin fire handle for async hook execution.
    plugin_fire: std::sync::Arc<dyn crate::feat::workflow::PluginFire>,

    pending_before_turn: HashMap<crate::protocol::SessionId, BeforeTurnMode>,
    /// Queue of remaining BeforeTurn attachments for sequential execution.
    /// Key: session_id, Value: ordered list of (AttachedWorkflow, BeforeTurnMode) pairs.
    before_turn_queue:
        HashMap<crate::protocol::SessionId, Vec<(AttachedWorkflow, BeforeTurnMode)>>,
}

/// Dependencies for [`WorkflowControllerActor`].
pub struct WorkflowControllerActorDeps {
    /// Shared application state.
    pub state: State,
    /// Runtime services.
    pub services: Services,
    /// Plugin system handle for async hook firing.
    pub async_plugins: std::sync::Arc<dyn crate::feat::workflow::PluginFire>,
}

impl Actor for WorkflowControllerActor {
    type Message = NoDirectMsg;
    type Deps = WorkflowControllerActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<AttachWorkflow>();
        ctx.subscribe_command::<DetachWorkflow>();
        ctx.subscribe_command::<ToggleWorkflow>();
        ctx.subscribe_command::<TriggerWorkflow>();
        ctx.subscribe_command::<FireBeforeTurn>();
        ctx.subscribe_event::<SessionPhaseChanged>();

        ctx.set_description("Orchestrates attached workflow lifecycle");

        let domain_ctx = Arc::new(DomainNodeContext::new(
            deps.services.clone(),
            deps.state.clone(),
        ));

        let actor = Self {
            ctx: domain_ctx,
            state: deps.state.clone(),
            services: deps.services,
            plugin_fire: deps.async_plugins,
            pending_before_turn: HashMap::new(),
            before_turn_queue: HashMap::new(),
        };

        // Rehydrate: reset any Running workflows back to Ready.
        // On restart, in-flight workflows from previous sessions are stale.
        {
            let guard = deps.state.read();
            let ids_to_reset: Vec<_> = guard
                .session
                .iter()
                .filter(|(_, session)| {
                    session
                        .core
                        .attached_workflows
                        .iter()
                        .any(|aw| matches!(aw.state, AttachedWorkflowState::Running))
                })
                .map(|(id, _)| id.clone())
                .collect();
            drop(guard);

            let mut guard = deps.state.write();
            for id in ids_to_reset {
                if let Some(session) = guard.session.get_mut(&id) {
                    for aw in &mut session.core.attached_workflows {
                        if matches!(aw.state, AttachedWorkflowState::Running) {
                            aw.state = AttachedWorkflowState::Ready;
                        }
                    }
                }
            }
        }

        actor
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, _ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(cmd) => self.handle_command(&cmd).await,
            ActorEnvelope::Event(Event::SessionPhaseChanged(ref payload)) => {
                self.handle_session_phase_changed(payload).await;
            }
            _ => {}
        }
    }
}

impl WorkflowControllerActor {
    /// Dispatches a command to the appropriate handler.
    async fn handle_command(&mut self, cmd: &Command) {
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
                self.handle_trigger_workflow(payload).await;
            }
            Command::FireBeforeTurn(payload) => {
                self.handle_fire_before_turn(payload).await;
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

        // No workflow_executions tracking in new plugin system.
        // Cancellation is handled by the plugin system itself.

        // Remove from session.
        {
            let mut guard = self.state.write();
            let Some(session) = guard.session.get_mut(session_id) else {
                return;
            };
            session
                .core
                .attached_workflows
                .retain(|aw| aw.id != *workflow_id);
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
    async fn handle_trigger_workflow(&mut self, payload: &TriggerWorkflow) {
        let session_id = payload.session_id.clone();
        let workflow_id = payload.workflow_id.clone();

        // Find the attachment.
        let attachment = {
            let guard = self.state.read();
            let Some(session) = guard.session.get(&session_id) else {
                return;
            };
            session
                .core
                .attached_workflows
                .iter()
                .find(|aw| {
                    aw.id == workflow_id
                        && matches!(aw.trigger, WorkflowTrigger::Manual)
                        && aw.enabled
                        && matches!(aw.state, AttachedWorkflowState::Ready)
                })
                .cloned()
        };

        let Some(attachment) = attachment else {
            return;
        };

        let wf_id = attachment.id.clone();

        // Set state to Running + begin busy — single write lock.
        {
            let mut guard = self.state.write();
            if let Some(session) = guard.session.get_mut(&session_id) {
                session.core.ephemeral.busy_count += 1;
                for aw in &mut session.core.attached_workflows {
                    if aw.id == wf_id {
                        aw.state = AttachedWorkflowState::Running;
                        break;
                    }
                }
            }
        }

        // Spawn execution and await result.
        let handle = self.spawn_attached_workflow(&session_id, attachment);
        let result = match handle.await {
            Ok(ok) => ok,
            Err(e) => Err(format!("join error: {e}")),
        };

        // Apply result — single write lock inside.
        self.apply_workflow_result(&session_id, &wf_id, result);
    }

    /// Handle `FireBeforeTurn` — fire all BeforeTurn workflows for the session,
    /// then merge the output with the deferred user text and either auto-send or put back.
    async fn handle_fire_before_turn(&mut self, payload: &FireBeforeTurn) {
        let session_id = payload.session_id.clone();

        // Collect all enabled BeforeTurn attachments in Ready state.
        let mut before_turn_attachments: Vec<_> = {
            let guard = self.state.read();
            let Some(session) = guard.session.get(&session_id) else {
                return;
            };
            session
                .core
                .attached_workflows
                .iter()
                .filter(|aw| {
                    aw.enabled
                        && matches!(aw.state, AttachedWorkflowState::Ready)
                        && matches!(aw.trigger, WorkflowTrigger::BeforeTurn(_))
                })
                .cloned()
                .collect()
        };

        if before_turn_attachments.is_empty() {
            return;
        }

        // Sort by attachment order to ensure deterministic sequence.
        before_turn_attachments.sort_by_key(|aw| aw.id.clone());

        // Take the first attachment to fire now, queue the rest.
        let first = before_turn_attachments.remove(0);
        let workflow_id = first.id.clone();
        let before_turn_mode = match &first.trigger {
            WorkflowTrigger::BeforeTurn(mode) => mode.clone(),
            _ => return,
        };

        let remaining: Vec<_> = before_turn_attachments
            .into_iter()
            .filter_map(|aw| {
                let mode = match &aw.trigger {
                    WorkflowTrigger::BeforeTurn(mode) => mode.clone(),
                    _ => return None,
                };
                Some((aw, mode))
            })
            .collect();

        // Store queue in actor field — no lock needed.
        if !remaining.is_empty() {
            self.before_turn_queue.insert(session_id.clone(), remaining);
        }

        // Store mode in actor field — no lock needed.
        self.pending_before_turn
            .insert(session_id.clone(), before_turn_mode);

        // Mark as Running + increment busy_count (single write lock).
        {
            let mut guard = self.state.write();
            if let Some(session) = guard.session.get_mut(&session_id) {
                for aw in &mut session.core.attached_workflows {
                    if aw.id == workflow_id {
                        aw.state = AttachedWorkflowState::Running;
                        break;
                    }
                }
                session.core.ephemeral.busy_count += 1;
            }
        }

        // Spawn + await + apply result.
        let handle = self.spawn_attached_workflow(&session_id, first);
        let result = match handle.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(format!("join error: {e}")),
        };
        self.apply_workflow_result(&session_id, &workflow_id, result);
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

            // Set state to Running + begin busy — single write lock.
            {
                let mut guard = self.state.write();
                if let Some(session) = guard.session.get_mut(session_id) {
                    session.core.ephemeral.busy_count += 1;
                    for aw in &mut session.core.attached_workflows {
                        if aw.id == wf_id {
                            aw.state = AttachedWorkflowState::Running;
                            break;
                        }
                    }
                }
            }

            // Spawn execution.
            let handle = self.spawn_attached_workflow(session_id, attachment);
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

        // Apply results in order (each call uses a single write lock).
        for (wf_id, result) in results {
            self.apply_workflow_result(session_id, &wf_id, result);
        }
    }

    /// Spawn an attached workflow using Lua.
    ///
    /// Returns the raw `spawn_lua_workflow` handle. The caller is responsible
    /// for awaiting and applying the result. No wrapper task — no double execution.
    fn spawn_attached_workflow(
        &self,
        session_id: &crate::protocol::SessionId,
        attachment: AttachedWorkflow,
    ) -> tokio::task::JoinHandle<Result<(), String>> {
        self.spawn_lua_workflow(
            &attachment.config,
            session_id,
            &attachment.id,
            &attachment.trigger,
        )
    }

    /// Spawns a plugin hook fire for the given attached workflow.
    ///
    /// Uses the new plugin system: fires `on_turn_end` through the
    /// `PluginFire` trait. The plugin system handles script loading,
    /// ctx building, and capability execution.
    fn spawn_lua_workflow(
        &self,
        config: &WorkflowConfig,
        session_id: &crate::protocol::SessionId,
        workflow_id: &WorkflowId,
        trigger: &WorkflowTrigger,
    ) -> tokio::task::JoinHandle<Result<(), String>> {
        let hook_name = match trigger {
            WorkflowTrigger::TurnEnd | WorkflowTrigger::TurnEndOneShot => "on_turn_end",
            WorkflowTrigger::BeforeTurn(_) => "on_before_turn",
            WorkflowTrigger::Manual => "on_manual_trigger",
        };

        // Get last assistant message for ctx data.
        let last_assistant_message = {
            let guard = self.state.read();
            guard
                .session
                .get(session_id)
                .and_then(|session| {
                    session
                        .history()
                        .iter()
                        .rev()
                        .find_map(|entry| match &entry.kind {
                            ChatEntryKind::Assistant(text) => Some(text.clone()),
                            _ => None,
                        })
                })
                .unwrap_or_default()
        };

        let ctx = serde_json::json!({
            "last_assistant_message": last_assistant_message,
            "session_id": session_id.to_string(),
            "workflow_id": workflow_id.to_string(),
        });

        let plugin_fire = self.plugin_fire.clone();
        let script_name = config.script.clone();

        tokio::spawn(async move {
            tracing::debug!(script = %script_name, hook = hook_name, "firing plugin hook");
            plugin_fire
                .fire_async_json(hook_name, &ctx)
                .await
                .map_err(|e| {
                    tracing::error!(script = %script_name, err = %e, "plugin hook failed");
                    e
                })
        })
    }

    /// Apply a workflow result to the session.
    ///
    /// Uses a **single write lock** for all AppState mutations.
    /// Ephemeral state (workflow_executions, pending_before_turn, before_turn_queue)
    /// is accessed through actor fields with no locking.
    fn apply_workflow_result(
        &mut self,
        session_id: &crate::protocol::SessionId,
        workflow_id: &WorkflowId,
        result: Result<(), String>,
    ) {
        // Workflow state cleanup handled by spawn_workflow callback.

        match result {
            Ok(()) => {
                // Lua workflows apply side effects via LuaHostHandler.
                // Read ephemeral state from actor fields (no lock).
                let before_turn_mode = self.pending_before_turn.remove(session_id);

                // Single write lock for all AppState mutations.
                let mut guard = self.state.write();
                if let Some(session) = guard.session.get_mut(session_id) {
                    for aw in &mut session.core.attached_workflows {
                        if aw.id == *workflow_id {
                            aw.state = AttachedWorkflowState::Completed;
                            break;
                        }
                    }
                    // Decrement busy_count inside the same lock.
                    session.core.ephemeral.busy_count =
                        session.core.ephemeral.busy_count.saturating_sub(1);
                }

                // --- BeforeTurn post-processing ---
                if let Some(mode) = before_turn_mode {
                    let original = guard
                        .session
                        .get_mut(session_id)
                        .and_then(|s| s.core.ephemeral.pending_user_text.take())
                        .unwrap_or_default();

                    let enhanced_text = String::new();
                    let merged = match &mode {
                        BeforeTurnMode::AutoSend { strategy }
                        | BeforeTurnMode::PutBack { strategy } => match strategy {
                            PromptMergeStrategy::Replace => enhanced_text.clone(),
                            PromptMergeStrategy::Prepend => format!("{enhanced_text}\n{original}"),
                            PromptMergeStrategy::Append => format!("{original}\n{enhanced_text}"),
                        },
                    };

                    // Check actor field for next BeforeTurn — no lock.
                    let next = self
                        .before_turn_queue
                        .get_mut(session_id)
                        .and_then(|queue| {
                            if queue.is_empty() {
                                None
                            } else {
                                Some(queue.remove(0))
                            }
                        });

                    if let Some((next_aw, next_mode)) = next {
                        // Store merged text — guard already held.
                        if let Some(session) = guard.session.get_mut(session_id) {
                            session.core.ephemeral.pending_user_text = Some(merged);
                        }
                        // Store next mode in actor field — no lock.
                        self.pending_before_turn
                            .insert(session_id.clone(), next_mode);
                        drop(guard); // Release before spawning.
                        self.spawn_attached_workflow(session_id, next_aw);
                    } else {
                        // No more in queue — clean up actor field.
                        self.before_turn_queue.remove(session_id);
                        drop(guard); // Release before sending commands.

                        match &mode {
                            BeforeTurnMode::AutoSend { .. } => {
                                let entry = ChatEntry::user_expanded(&merged, &merged);
                                self.ctx.send_command(Command::EnqueueUserMessage(
                                    EnqueueUserMessage {
                                        session_id: session_id.clone(),
                                        entry,
                                    },
                                ));
                            }
                            BeforeTurnMode::PutBack { .. } => {
                                self.ctx.send_command(Command::SetChatInputText(
                                    SetChatInputText {
                                        session_id: session_id.clone(),
                                        text: merged,
                                    },
                                ));
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

                // Graceful degradation: read ephemeral state from actor fields.
                let had_before_turn = self.pending_before_turn.remove(session_id).is_some();

                // Single write lock for all AppState mutations.
                let mut guard = self.state.write();
                if let Some(session) = guard.session.get_mut(session_id) {
                    for aw in &mut session.core.attached_workflows {
                        if aw.id == *workflow_id {
                            aw.state = AttachedWorkflowState::Failed {
                                reason: reason.clone(),
                            };
                            break;
                        }
                    }
                    session.push_entry(ChatEntry::system(&format!("[Workflow] Failed: {reason}")));
                    // Decrement busy_count inside the same lock.
                    session.core.ephemeral.busy_count =
                        session.core.ephemeral.busy_count.saturating_sub(1);
                }

                if had_before_turn {
                    let original = guard
                        .session
                        .get_mut(session_id)
                        .and_then(|s| s.core.ephemeral.pending_user_text.take());

                    // Clean up actor field — no lock.
                    self.before_turn_queue.remove(session_id);
                    drop(guard); // Release before sending commands.

                    if let Some(original) = original {
                        let entry = ChatEntry::user_expanded(&original, &original);
                        self.ctx
                            .send_command(Command::EnqueueUserMessage(EnqueueUserMessage {
                                session_id: session_id.clone(),
                                entry,
                            }));
                    }
                }
            }
        }
    }
}
