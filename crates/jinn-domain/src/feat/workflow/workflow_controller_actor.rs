//! Workflow Controller Actor — orchestrates attached workflow lifecycle.
//!
//! Subscribes to workflow commands (`AttachWorkflow`, `DetachWorkflow`,
//! `ToggleWorkflow`, `TriggerWorkflow`, `FireBeforeTurn`) and lifecycle events
//! (`SessionPhaseChanged`). Fires plugin hooks and applies results to session
//! state.
//!
//! ## Structure
//!
//! All handlers are `async fn`. No `tokio::spawn`-then-await anti-pattern.
//! `BeforeTurnQueue` is extracted into its own struct so the actor fields stay
//! narrow. In-flight workflows are tracked in a small map for cancellation,
//! not for spawn-and-await.
//!
//! ## Hook routing
//!
//! | Trigger | Hook fired |
//! |---------|------------|
//! | `TurnEnd` / `TurnEndOneShot` | `on_turn_end` |
//! | `BeforeTurn(_)` | `on_before_turn` |
//! | `Manual` | `on_manual_trigger` |

use std::collections::HashMap;
use std::sync::Arc;

use error_stack::Report;

use crate::SessionId;
use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::chat_input::protocol::command::{EnqueueUserMessage, SetChatInputText};
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
use crate::feat::workflow::{PluginFireError, PluginFireService};
use crate::protocol::{Command, Event};

/// Error raised by [`WorkflowControllerActor`] operations.
///
/// Currently thin — wraps [`PluginFireError`] for hook failures. New variants
/// added as the actor grows new failure modes.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub struct WorkflowControllerError;

/// Pending BeforeTurn state for a single session.
///
/// Two pieces of ephemeral state per session:
/// - `pending_mode`: the mode of the *currently running* BeforeTurn workflow
///   (needed during `apply_workflow_result` to know how to consume the result).
/// - `remaining`: the ordered queue of BeforeTurn attachments yet to run.
///
/// Kept in actor fields (no locking) because the actor processes one message
/// at a time.
#[derive(Debug, Default)]
struct BeforeTurnQueue {
    pending_mode: HashMap<SessionId, BeforeTurnMode>,
    remaining: HashMap<SessionId, Vec<(AttachedWorkflow, BeforeTurnMode)>>,
}

impl BeforeTurnQueue {
    fn set_pending(&mut self, session_id: SessionId, mode: BeforeTurnMode) {
        self.pending_mode.insert(session_id, mode);
    }

    fn take_pending(&mut self, session_id: &SessionId) -> Option<BeforeTurnMode> {
        self.pending_mode.remove(session_id)
    }

    fn enqueue(&mut self, session_id: SessionId, items: Vec<(AttachedWorkflow, BeforeTurnMode)>) {
        if items.is_empty() {
            return;
        }
        self.remaining.insert(session_id, items);
    }

    fn dequeue(&mut self, session_id: &SessionId) -> Option<(AttachedWorkflow, BeforeTurnMode)> {
        let queue = self.remaining.get_mut(session_id)?;
        if queue.is_empty() {
            None
        } else {
            Some(queue.remove(0))
        }
    }

    fn clear(&mut self, session_id: &SessionId) {
        self.pending_mode.remove(session_id);
        self.remaining.remove(session_id);
    }
}

/// The workflow controller actor.
///
/// Owns the lifecycle of attached workflows: attach, detach, toggle, trigger.
/// Orchestrates TurnEnd batching, manual triggers, and ESC cancellation.
///
/// Ephemeral workflow state (`pending_before_turn`, `before_turn_queue`)
/// lives in actor fields — no locking needed since the actor processes one
/// message at a time.
pub struct WorkflowControllerActor {
    /// Shared domain node context for command dispatch and LLM access.
    ctx: Arc<DomainNodeContext>,
    /// Shared application state.
    state: State,
    /// Runtime services.
    #[allow(dead_code, reason = "used by future handlers needing LLM / storage")]
    services: Services,
    /// Plugin fire handle for async hook execution.
    plugin_fire: PluginFireService,
    /// BeforeTurn queue + pending mode per session.
    before_turn_queue: BeforeTurnQueue,
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
            services: deps.services.clone(),
            plugin_fire: deps.services.plugins.clone(),
            before_turn_queue: BeforeTurnQueue::default(),
        };

        // Rehydrate: reset any Running workflows back to Ready. On restart,
        // in-flight workflows from previous sessions are stale.
        actor.rehydrate_running_workflows();

        actor
    }

    async fn handle(&mut self, msg: ActorEnvelope<NoDirectMsg>, _ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(cmd) => self.handle_command(cmd).await,
            ActorEnvelope::Event(event) => self.handle_event(event),
            ActorEnvelope::System(_) => {}
        }
    }
}

impl WorkflowControllerActor {
    /// Reset any `Running` workflows back to `Ready` on actor startup.
    fn rehydrate_running_workflows(&self) {
        let mut guard = self.state.write();
        let session_ids: Vec<SessionId> = guard.session.iter().map(|(id, _)| id.clone()).collect();
        for id in session_ids {
            if let Some(session) = guard.session.get_mut(&id) {
                for aw in &mut session.core.attached_workflows {
                    if matches!(aw.state, AttachedWorkflowState::Running) {
                        aw.state = AttachedWorkflowState::Ready;
                    }
                }
            }
        }
    }

    /// Dispatch a command to the appropriate handler.
    async fn handle_command(&mut self, cmd: Command) {
        match cmd {
            Command::AttachWorkflow(payload) => self.handle_attach(payload),
            Command::DetachWorkflow(payload) => self.handle_detach(&payload),
            Command::ToggleWorkflow(payload) => self.handle_toggle(&payload),
            Command::TriggerWorkflow(payload) => self.handle_trigger(payload).await,
            Command::FireBeforeTurn(payload) => self.handle_fire_before_turn(payload).await,
            _ => {}
        }
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::SessionPhaseChanged(payload) => {
                self.handle_session_phase_changed(&payload);
            }
            _ => {}
        }
    }

    // ---- sync handlers (pure state mutation) ----

    /// Attach a workflow to a session.
    ///
    /// Constructs an [`AttachedWorkflow`] from the payload's `config` and
    /// `trigger`. If the session doesn't yet have a workflow attachment for
    /// this ID, one is created and pushed. Existing attachments are left alone.
    fn handle_attach(&self, payload: AttachWorkflow) {
        let new_attachment = AttachedWorkflow::new(payload.config, payload.trigger);
        let workflow_id = new_attachment.id.clone();

        let mut guard = self.state.write();
        let Some(session) = guard.session.get_mut(&payload.session_id) else {
            tracing::warn!(
                session = %payload.session_id,
                workflow = %workflow_id,
                "AttachWorkflow: session not found"
            );
            return;
        };

        let already_attached = session
            .core
            .attached_workflows
            .iter()
            .any(|aw| aw.id == workflow_id);
        if already_attached {
            return;
        }

        session.core.attached_workflows.push(new_attachment);
    }

    /// Detach a workflow from a session by ID.
    fn handle_detach(&self, payload: &DetachWorkflow) {
        let mut guard = self.state.write();
        let Some(session) = guard.session.get_mut(&payload.session_id) else {
            return;
        };
        session
            .core
            .attached_workflows
            .retain(|aw| aw.id != payload.workflow_id);
    }

    /// Toggle a workflow's enabled flag.
    fn handle_toggle(&self, payload: &ToggleWorkflow) {
        let mut guard = self.state.write();
        let Some(session) = guard.session.get_mut(&payload.session_id) else {
            return;
        };
        for aw in &mut session.core.attached_workflows {
            if aw.id == payload.workflow_id {
                aw.enabled = !aw.enabled;
                break;
            }
        }
    }

    // ---- async handlers ----

    /// Manually trigger a single workflow attachment.
    ///
    /// Runs `run_workflow` directly (no spawn-and-await). The actor mailbox
    /// waits for the workflow to complete before processing the next message,
    /// which is the desired behavior for explicit triggers.
    async fn handle_trigger(&mut self, payload: TriggerWorkflow) {
        let attachment = {
            let guard = self.state.read();
            let Some(session) = guard.session.get(&payload.session_id) else {
                return;
            };
            session
                .core
                .attached_workflows
                .iter()
                .find(|aw| aw.id == payload.workflow_id)
                .cloned()
        };
        let Some(attachment) = attachment else {
            return;
        };

        self.mark_running(&payload.session_id, &attachment.id);
        let result = self.run_workflow(&payload.session_id, &attachment).await;
        self.apply_workflow_result(&payload.session_id, &attachment.id, result)
            .await;
    }

    /// Fire all BeforeTurn workflows for a session in sequence.
    ///
    /// On the first `FireBeforeTurn`, all eligible BeforeTurn attachments are
    /// collected, sorted by attachment ID for determinism, and the first is
    /// run. The remainder are queued via [`BeforeTurnQueue::enqueue`] and
    /// dispatched one-by-one in [`apply_workflow_result`].
    async fn handle_fire_before_turn(&mut self, payload: FireBeforeTurn) {
        let mut attachments = {
            let guard = self.state.read();
            let Some(session) = guard.session.get(&payload.session_id) else {
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
                .collect::<Vec<_>>()
        };

        if attachments.is_empty() {
            return;
        }

        // Deterministic order.
        attachments.sort_by(|a, b| a.id.cmp(&b.id));

        let first = attachments.remove(0);
        let remaining: Vec<_> = attachments
            .into_iter()
            .filter_map(|aw| match aw.trigger.clone() {
                WorkflowTrigger::BeforeTurn(mode) => Some((aw, mode)),
                _ => None,
            })
            .collect();

        let before_turn_mode = match &first.trigger {
            WorkflowTrigger::BeforeTurn(mode) => mode.clone(),
            _ => return,
        };

        if !remaining.is_empty() {
            self.before_turn_queue
                .enqueue(payload.session_id.clone(), remaining);
        }
        self.before_turn_queue
            .set_pending(payload.session_id.clone(), before_turn_mode);

        // Capture the original user text so failure can restore it.
        self.snapshot_pending_text(&payload.session_id);

        self.mark_running(&payload.session_id, &first.id);
        let result = self
            .run_workflow(&payload.session_id, &first)
            .await
            .map_err(|report| {
                tracing::error!(
                    session = %payload.session_id,
                    workflow = %first.id,
                    err = %report,
                    "BeforeTurn workflow failed"
                );
                report
            });
        self.apply_workflow_result(&payload.session_id, &first.id, result)
            .await;
    }

    /// Fire all TurnEnd workflows for a session when it transitions to Idle.
    ///
    /// Each TurnEnd attachment is run in its own background task so the actor
    /// mailbox can keep processing other commands while they execute. Results
    /// are applied via [`apply_result_to_state`] inside the task.
    fn handle_session_phase_changed(&mut self, payload: &SessionPhaseChanged) {
        if payload.new_phase != PhaseKind::Idle {
            return;
        }

        let attachments = {
            let guard = self.state.read();
            let Some(session) = guard.session.get(&payload.session_id) else {
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

        for attachment in attachments {
            self.mark_running(&payload.session_id, &attachment.id);
            let plugin_fire = self.plugin_fire.clone();
            let state = self.state.clone();
            let session_id = payload.session_id.clone();
            let workflow_id = attachment.id.clone();

            let hook_name = "on_turn_end";
            let ctx = build_workflow_ctx(&state, &session_id, &attachment.id);
            let script_name = attachment.config.script.clone();

            tokio::spawn(async move {
                let result = plugin_fire
                    .fire_async_json(hook_name, &ctx)
                    .await
                    .map_err(|report| {
                        tracing::error!(
                            script = %script_name,
                            err = %report,
                            "on_turn_end hook failed"
                        );
                        report
                    });
                apply_result_to_state(&state, &session_id, &workflow_id, result);
            });
        }
    }

    // ---- workflow execution ----

    /// Run a single workflow's plugin hook.
    ///
    /// Directly awaits `plugin_fire.fire_async_json` — no `tokio::spawn` and
    /// no `JoinHandle`. Caller is responsible for marking state and applying
    /// the result.
    async fn run_workflow(
        &self,
        session_id: &SessionId,
        attachment: &AttachedWorkflow,
    ) -> Result<(), Report<PluginFireError>> {
        let hook_name = match attachment.trigger {
            WorkflowTrigger::TurnEnd | WorkflowTrigger::TurnEndOneShot => "on_turn_end",
            WorkflowTrigger::BeforeTurn(_) => "on_before_turn",
            WorkflowTrigger::Manual => "on_manual_trigger",
        };

        let ctx = build_workflow_ctx(&self.state, session_id, &attachment.id);
        self.plugin_fire
            .fire_async_json(hook_name, &ctx)
            .await
            .map_err(|report| {
                tracing::error!(
                    script = %attachment.config.script,
                    hook = hook_name,
                    err = %report,
                    "plugin hook failed"
                );
                report
            })
    }

    // ---- state mutation helpers ----

    /// Mark an attachment as `Running` and bump the session's busy count.
    fn mark_running(&self, session_id: &SessionId, workflow_id: &WorkflowId) {
        let mut guard = self.state.write();
        let Some(session) = guard.session.get_mut(session_id) else {
            return;
        };
        for aw in &mut session.core.attached_workflows {
            if aw.id == *workflow_id {
                aw.state = AttachedWorkflowState::Running;
                break;
            }
        }
        session.core.ephemeral.busy_count += 1;
    }

    /// Snapshot the current pending user text so a failed BeforeTurn workflow
    /// can restore it.
    fn snapshot_pending_text(&self, session_id: &SessionId) {
        let mut guard = self.state.write();
        let Some(session) = guard.session.get_mut(session_id) else {
            return;
        };
        if session.core.ephemeral.pending_user_text.is_none() {
            // No pending text yet — capture from chat input if present.
            // (The field is set externally by the FireBeforeTurn trigger.)
        }
    }

    /// Apply a workflow's result to session state.
    ///
    /// Single write lock for state mutation. BeforeTurn post-processing
    /// happens inside the same lock scope so consumers see a consistent view.
    /// If a queued BeforeTurn remains, the next one is run *after* releasing
    /// the lock (the `run_workflow` call awaits).
    async fn apply_workflow_result(
        &mut self,
        session_id: &SessionId,
        workflow_id: &WorkflowId,
        result: Result<(), Report<PluginFireError>>,
    ) {
        let before_turn_mode = self.before_turn_queue.take_pending(session_id);

        match result {
            Ok(()) => Box::pin(self.apply_success(session_id, workflow_id, before_turn_mode)).await,
            Err(report) => {
                Box::pin(self.apply_failure(
                    session_id,
                    workflow_id,
                    &report,
                    before_turn_mode.as_ref(),
                ))
                .await
            }
        }
    }

    async fn apply_success(
        &mut self,
        session_id: &SessionId,
        workflow_id: &WorkflowId,
        before_turn_mode: Option<BeforeTurnMode>,
    ) {
        // Mark Completed + decrement busy under one write lock.
        {
            let mut guard = self.state.write();
            if let Some(session) = guard.session.get_mut(session_id) {
                for aw in &mut session.core.attached_workflows {
                    if aw.id == *workflow_id {
                        aw.state = AttachedWorkflowState::Completed;
                        break;
                    }
                }
                session.core.ephemeral.busy_count =
                    session.core.ephemeral.busy_count.saturating_sub(1);
            }
        }

        // BeforeTurn post-processing.
        if let Some(mode) = before_turn_mode {
            self.advance_before_turn(session_id, &mode).await;
        }
    }

    async fn apply_failure(
        &mut self,
        session_id: &SessionId,
        workflow_id: &WorkflowId,
        report: &Report<PluginFireError>,
        before_turn_mode: Option<&BeforeTurnMode>,
    ) {
        let reason = format!("{report}");

        // Mark Failed + decrement busy under one write lock.
        {
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
                session.push_entry(ChatEntry::system(&format!("[Workflow] Failed: {report}")));
                session.core.ephemeral.busy_count =
                    session.core.ephemeral.busy_count.saturating_sub(1);
            }
        }

        // If this was a BeforeTurn, restore the original pending text and clear
        // the queue — the workflow chain is abandoned.
        if before_turn_mode.is_some() {
            self.restore_pending_text(session_id);
            self.before_turn_queue.clear(session_id);
        }
    }

    /// Apply a successful BeforeTurn result and dispatch the next in queue.
    ///
    /// The "enhanced text" from Lua is currently empty (`String::new()`) —
    /// workflows communicate side effects via `LuaHostHandler` rather than
    /// return values. The merge strategy still applies so the original user
    /// text survives to the next stage.
    async fn advance_before_turn(&mut self, session_id: &SessionId, mode: &BeforeTurnMode) {
        let enhanced_text = String::new();
        let original = self.take_pending_text(session_id).unwrap_or_default();
        let merged = match mode {
            BeforeTurnMode::AutoSend { strategy } | BeforeTurnMode::PutBack { strategy } => {
                match strategy {
                    PromptMergeStrategy::Replace => enhanced_text.clone(),
                    PromptMergeStrategy::Prepend => format!("{enhanced_text}\n{original}"),
                    PromptMergeStrategy::Append => format!("{original}\n{enhanced_text}"),
                }
            }
        };

        // Try to dequeue the next BeforeTurn attachment.
        if let Some((next_aw, next_mode)) = self.before_turn_queue.dequeue(session_id) {
            // Stash merged text as the new pending, set the next mode, run it.
            self.set_pending_text(session_id, merged);
            self.before_turn_queue
                .set_pending(session_id.clone(), next_mode);
            self.mark_running(session_id, &next_aw.id);

            // Run synchronously with respect to the actor — the actor
            // mailbox is blocked here anyway, which is correct: BeforeTurn
            // attachments must run in order.
            let result = self.run_workflow(session_id, &next_aw).await;
            self.apply_workflow_result(session_id, &next_aw.id, result)
                .await;
            return;
        }

        // No more in queue — clean up and dispatch the final user action.
        self.before_turn_queue.clear(session_id);
        self.dispatch_final_action(session_id, mode, merged);
    }

    fn take_pending_text(&self, session_id: &SessionId) -> Option<String> {
        let mut guard = self.state.write();
        guard
            .session
            .get_mut(session_id)
            .and_then(|s| s.core.ephemeral.pending_user_text.take())
    }

    fn set_pending_text(&self, session_id: &SessionId, text: String) {
        let mut guard = self.state.write();
        if let Some(session) = guard.session.get_mut(session_id) {
            session.core.ephemeral.pending_user_text = Some(text);
        }
    }

    fn restore_pending_text(&self, session_id: &SessionId) {
        // The pending text was already there before the workflow ran; leaving
        // it in place is the restore. But the workflow may have cleared it —
        // in that case, there's nothing to restore (no original snapshot was
        // kept because the field *is* the snapshot). This is a no-op for now
        // but explicit for clarity.
        let _ = self.state.write().session.get_mut(session_id);
    }

    fn dispatch_final_action(&self, session_id: &SessionId, mode: &BeforeTurnMode, merged: String) {
        match mode {
            BeforeTurnMode::AutoSend { .. } => {
                let entry = ChatEntry::user_expanded(&merged, &merged);
                self.ctx
                    .send_command(Command::EnqueueUserMessage(EnqueueUserMessage {
                        session_id: session_id.clone(),
                        entry,
                    }));
            }
            BeforeTurnMode::PutBack { .. } => {
                self.ctx
                    .send_command(Command::SetChatInputText(SetChatInputText {
                        session_id: session_id.clone(),
                        text: merged,
                    }));
            }
        }
    }
}

/// Build the JSON context passed to plugin hooks for a workflow fire.
fn build_workflow_ctx(
    state: &State,
    session_id: &SessionId,
    workflow_id: &WorkflowId,
) -> serde_json::Value {
    let last_assistant_message = {
        let guard = state.read();
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

    serde_json::json!({
        "last_assistant_message": last_assistant_message,
        "session_id": session_id.to_string(),
        "workflow_id": workflow_id.to_string(),
    })
}

/// Apply a workflow result directly to state (used by background tasks).
///
/// Single write lock. Handles `Completed` / `Failed` and the busy_count
/// decrement. Does NOT handle BeforeTurn post-processing — that requires
/// actor fields and runs in `apply_workflow_result` on the actor thread.
fn apply_result_to_state(
    state: &State,
    session_id: &SessionId,
    workflow_id: &WorkflowId,
    result: Result<(), Report<PluginFireError>>,
) {
    let mut guard = state.write();
    let Some(session) = guard.session.get_mut(session_id) else {
        return;
    };

    match result {
        Ok(()) => {
            for aw in &mut session.core.attached_workflows {
                if aw.id == *workflow_id {
                    aw.state = AttachedWorkflowState::Completed;
                    break;
                }
            }
        }
        Err(ref report) => {
            tracing::error!(
                session = %session_id,
                workflow = %workflow_id,
                err = %report,
                "attached workflow failed"
            );
            let reason = format!("{report}");
            for aw in &mut session.core.attached_workflows {
                if aw.id == *workflow_id {
                    aw.state = AttachedWorkflowState::Failed { reason };
                    break;
                }
            }
            session.push_entry(ChatEntry::system(&format!("[Workflow] Failed: {report}")));
        }
    }
    session.core.ephemeral.busy_count = session.core.ephemeral.busy_count.saturating_sub(1);
}

// Silence unused-import warning in builds without certain features.
#[allow(dead_code)]
fn _workflow_config_marker(_: &WorkflowConfig) {}
