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
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind};

use crate::feat::session::phase_machine::PhaseKind;

use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;

use crate::feat::workflow::attached_workflow::{
    AttachedWorkflow, AttachedWorkflowState, BeforeTurnMode, PromptMergeStrategy, WorkflowConfig,
    WorkflowTrigger,
};

use crate::feat::workflow::domain_node_context::DomainNodeContext;
use crate::feat::workflow::protocol::command::{
    AttachWorkflow, DetachWorkflow, FireBeforeTurn, ToggleWorkflow, TriggerWorkflow,
};

use crate::feat::chat_input::protocol::command::{EnqueueUserMessage, SetChatInputText};
use crate::feat::workflow::workflow_response::WorkflowResponse;
use crate::feat::workflow::workflow_state::{WorkflowExecutionState, WorkflowId};
use crate::protocol::{Command, Event};

use crate::feat::luaworkflow::host_handler::LuaHostHandler;
use jinn_lua_workflow::{spawn_one_shot, CtxConfig, HostRequest};

/// The workflow controller actor.
///
/// Owns the lifecycle of attached workflows: attach, detach, toggle, trigger.
/// Orchestrates TurnEnd batching, manual triggers, and ESC cancellation.
pub struct WorkflowControllerActor {
    /// Shared domain node context for LLM access.
    ctx: Arc<DomainNodeContext>,
    /// Shared application state.
    state: State,
    /// Runtime services.
    services: Services,
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

        let domain_ctx = Arc::new(DomainNodeContext::new(deps.services.clone(), deps.state.clone()));

        let actor = Self {
            ctx: domain_ctx,
            state: deps.state.clone(),
            services: deps.services,
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
            guard.pending_before_turn.clear();

            guard.before_turn_queue.clear();
        }

        actor
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
            Command::FireBeforeTurn(payload) => {
                self.handle_fire_before_turn(payload);
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
    fn handle_trigger_workflow(&mut self, payload: &TriggerWorkflow) {
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

        // Fire it.
        self.spawn_attached_workflow(&session_id, attachment);
    }

    /// Handle `FireBeforeTurn` — fire all BeforeTurn workflows for the session,
    /// then merge the output with the deferred user text and either auto-send or put back.
    fn handle_fire_before_turn(&mut self, payload: &FireBeforeTurn) {
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

        if !remaining.is_empty() {
            self.state
                .write()
                .before_turn_queue
                .insert(session_id.clone(), remaining);
        }

        // Store the mode for post-processing when the workflow completes.
        self.state
            .write()
            .pending_before_turn
            .insert(session_id.clone(), before_turn_mode);

        // Fire the first attachment.
        self.spawn_attached_workflow(&session_id, first);
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
                session.core.ephemeral.busy_count =
                    session.core.ephemeral.busy_count.saturating_sub(1);
            }
        }
    }

    /// Spawn an attached workflow execution and return a tokio JoinHandle.
    fn spawn_attached_workflow_tokio(
        &self,
        session_id: &crate::protocol::SessionId,
        attachment: AttachedWorkflow,
    ) -> tokio::task::JoinHandle<Result<Vec<WorkflowResponse>, String>> {
        if matches!(attachment.config, WorkflowConfig::Judge { .. }) {
            return self.spawn_lua_workflow(
                &attachment.config,
                session_id,
                &attachment.id,
                &attachment.trigger,
            );
        }

        // ── Existing node-graph path (unchanged) ──────────────────────────
        let workflow_id = attachment.id.clone();
        let session_id = session_id.clone();
        let state = self.state.clone();

        let graph = attachment.config.build_graph();
        let execution = Arc::new(jinn_workflow::execution::WorkflowExecution::new(graph));
        let cancel = CancellationToken::new();

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
            let result =
                jinn_workflow::engine::execute_with_cancel(execution, domain_ctx, cancel).await;

            match result {
                Ok(workflow_result) => {
                    let _ = workflow_result;
                    Ok(Vec::new())
                }
                Err(report) => Err(format!("{report:#}")),
            }
        })
    }

    /// Returns true if the config should use the Lua workflow path.
    fn is_lua_config(config: &WorkflowConfig) -> bool {
        matches!(config, WorkflowConfig::Judge { .. })
    }

    /// Spawn a Lua one-shot workflow for judge configurations.
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
        let services = self.services.clone();
        tokio::spawn(async move {
            let result = handle.await;

            let response = match result {
                Ok(Ok(outputs)) => Ok(outputs),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(format!("join error: {e}")),
            };

            let actor = WorkflowControllerActor { ctx, state, services };
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

    /// Spawns a Lua one-shot workflow for the given config.
    /// Reads the script from `res/plugins/{script_name}/init.lua`, builds
    /// a ctx table with capabilities, and runs it through `spawn_one_shot`.
    /// The host handler processes capability requests concurrently.
    fn spawn_lua_workflow(
        &self,
        config: &WorkflowConfig,
        session_id: &crate::protocol::SessionId,
        workflow_id: &WorkflowId,
        trigger: &WorkflowTrigger,
    ) -> tokio::task::JoinHandle<Result<Vec<WorkflowResponse>, String>> {
        // Determine script name from config.
        let script_name = match config {
            WorkflowConfig::Judge { script, .. } => script.clone(),
            other => {
                let label = other.label().to_owned();
                return tokio::spawn(async move {
                    Err(format!("unsupported Lua config: {label}"))
                });
            }
        };

        // Read the Lua script source.
        let script_source = {
            let plugins_dir = self.services.paths.plugins_dir();
            let user_path = plugins_dir.join(&script_name).join("init.lua");
            let system_plugins_dir = self.services.paths.system_plugins_dir();
            let system_path = system_plugins_dir.join(&script_name).join("init.lua");
            if user_path.is_file() {
                std::fs::read_to_string(&user_path)
            } else if system_path.is_file() {
                std::fs::read_to_string(&system_path)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("no init.lua for {script_name}"),
                ))
            }
            .map_err(|e| format!("read script: {e}"))
        };
        let script_source = match script_source {
            Ok(s) => s,
            Err(e) => {
                return tokio::spawn(async move { Err(e) });
            }
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

        // Build ctx config with capabilities.
        let ctx_config = CtxConfig::data_only(&serde_json::json!({
            "last_assistant_message": last_assistant_message,
            "session_id": session_id.to_string(),
        }))
        .with_push_user()
        .with_push_system()
        .with_turn_off()
        .session_id(session_id.to_string())
        .workflow_id(workflow_id.to_string());

        // Create channel for host requests.
        let (host_tx, host_rx) = kanal::unbounded::<HostRequest>();

        // Spawn the Lua VM.
        let mut vm_handle = spawn_one_shot(
            script_source,
            script_name.clone(),
            host_tx,
            ctx_config,
        );

        // Spawn handler task that processes host requests.
        let state = self.state.clone();
        let handler_sid = session_id.clone();
        let handler_wid = workflow_id.clone();
        let handler_ctx = self.ctx.clone();

        tokio::spawn(async move {
            let handler = LuaHostHandler::new(state.clone(), handler_ctx);

            // Process host requests until the VM completes.
            let mut vm_done = false;
            loop {
                if vm_done && host_rx.is_empty() {
                    break;
                }
                // Non-blocking drain of pending requests.
                while let Ok(Some(req)) = host_rx.try_recv() {
                    match req {
                        HostRequest::Shutdown => break,
                        request => {
                            handler.handle_request(request).await;

                        }
                    }
                }
                if vm_done {
                    break;
                }
                // Check if VM is done (non-blocking poll).
                // Use a small sleep to avoid busy-waiting.
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
                    result = &mut vm_handle => {
                        vm_done = true;
                        match result {
                            Ok(Ok(_)) => {
                                tracing::info!(script = %script_name, "lua workflow completed");
                            }
                            Ok(Err(e)) => {
                                tracing::error!(script = %script_name, err = %e, "lua workflow failed");
                            }
                            Err(e) => {
                                tracing::error!(script = %script_name, err = %e, "lua task panicked");
                            }
                        }
                    }
                }
            }

            // Side effects are already applied by the host handler.
            Ok(Vec::new())
        })
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
                let should_detach = responses
                    .iter()
                    .any(|r| matches!(r, WorkflowResponse::Detach));
                let should_turn_off = responses
                    .iter()
                    .any(|r| matches!(r, WorkflowResponse::TurnOff));

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
                            session
                                .core
                                .attached_workflows
                                .retain(|aw| aw.id != *workflow_id);
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

                // --- BeforeTurn post-processing ---
                // If this session has a pending BeforeTurn, merge the enhanced text
                // with the original user text. Then either fire the next queued
                // BeforeTurn attachment, or finalize with AutoSend/PutBack.
                if let Some(mode) = self.state.write().pending_before_turn.remove(session_id) {
                    // Extract enhanced text from workflow output.
                    let enhanced_text: String = responses
                        .iter()
                        .filter_map(|r| match r {
                            WorkflowResponse::PushSessionHistory(entry) => match &entry.kind {
                                crate::feat::session::chat_entry::ChatEntryKind::Assistant(
                                    text,
                                ) => Some(text.as_str()),
                                _ => None,
                            },
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");

                    let original = {
                        let mut guard = self.state.write();
                        guard
                            .session
                            .get_mut(session_id)
                            .and_then(|s| s.core.ephemeral.pending_user_text.take())
                            .unwrap_or_default()
                    };
                    let merged = match &mode {
                        BeforeTurnMode::AutoSend { strategy }
                        | BeforeTurnMode::PutBack { strategy } => match strategy {
                            PromptMergeStrategy::Replace => enhanced_text.clone(),
                            PromptMergeStrategy::Prepend => format!("{enhanced_text}\n{original}"),
                            PromptMergeStrategy::Append => format!("{original}\n{enhanced_text}"),
                        },
                    };

                    // Check if there's a next BeforeTurn in the queue.
                    let next = {
                        let mut guard = self.state.write();
                        guard
                            .before_turn_queue
                            .get_mut(session_id)
                            .and_then(|queue| {
                                if queue.is_empty() {
                                    None
                                } else {
                                    Some(queue.remove(0))
                                }
                            })
                    };

                    if let Some((next_aw, next_mode)) = next {
                        // Store merged text as pending_user_text for the next workflow.
                        {
                            let mut guard = self.state.write();
                            if let Some(session) = guard.session.get_mut(session_id) {
                                session.core.ephemeral.pending_user_text = Some(merged);
                            }
                        }
                        // Store the next mode and fire the next attachment.
                        self.state
                            .write()
                            .pending_before_turn
                            .insert(session_id.clone(), next_mode);
                        self.spawn_attached_workflow(session_id, next_aw);
                    } else {
                        // No more in queue — finalize.
                        // Clean up the queue entry.
                        self.state.write().before_turn_queue.remove(session_id);

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
                // Set Failed state, push system entry.
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
                }

                drop(guard);

                // Graceful degradation for BeforeTurn failures:
                // Fall back to sending the original user text.
                if self
                    .state
                    .write()
                    .pending_before_turn
                    .remove(session_id)
                    .is_some()
                {
                    let original = {
                        let mut guard = self.state.write();
                        guard
                            .session
                            .get_mut(session_id)
                            .and_then(|s| s.core.ephemeral.pending_user_text.take())
                    };
                    if let Some(original) = original {
                        let entry = ChatEntry::user_expanded(&original, &original);
                        self.ctx
                            .send_command(Command::EnqueueUserMessage(EnqueueUserMessage {
                                session_id: session_id.clone(),
                                entry,
                            }));
                    }
                    // Clean up any remaining queued BeforeTurn attachments.
                    self.state.write().before_turn_queue.remove(session_id);
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
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        dead_code,
        clippy::unwrap_used
    )]

    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::services::test_services::TestServices;
    use crate::common::services::Services;

    use crate::feat::session::chat_session::ChatSessionState;
    use crate::feat::workflow::attached_workflow::{
        AttachedWorkflowState, OneShotKind, WorkflowConfig, WorkflowTrigger,
    };
    use crate::feat::session::chat_entry::ChatEntryKind;
    use crate::protocol::SessionId;

    struct TestHarness {
        state: State,
        services: Services,
    }

    impl TestHarness {
        fn new() -> Self {
            let services = TestServices::builder().build();
            let state = State::new(AppState::default());
            Self { state, services }
        }

        fn insert_session(&self, session: ChatSessionState) -> SessionId {
            let id = session.session_id().clone();
            self.state.write().session.insert(session);
            id
        }

        fn session_has_attachment(&self, session_id: &SessionId, workflow_id: &WorkflowId) -> bool {
            let guard = self.state.read();
            guard.session.get(session_id).map_or(false, |s| {
                s.core
                    .attached_workflows
                    .iter()
                    .any(|aw| aw.id == *workflow_id)
            })
        }
    }

    // --- Test 18: controller_finds_turn_end_attachments_on_idle ---

    #[rstest::rstest]
    fn controller_finds_turn_end_attachments_on_idle() {
        let h = TestHarness::new();
        let mut session = ChatSessionState::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 3,
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
            },
            WorkflowTrigger::TurnEnd,
        );
        let wf_id = aw.id.clone();
        session.core.attached_workflows.push(aw);
        let session_id = h.insert_session(session);

        assert!(h.session_has_attachment(&session_id, &wf_id));
        let guard = h.state.read();
        let session = guard.session.get(&session_id).expect("session");
        let matching: Vec<_> = session
            .core
            .attached_workflows
            .iter()
            .filter(|aw| {
                aw.enabled
                    && matches!(aw.state, AttachedWorkflowState::Ready)
                    && matches!(aw.trigger, WorkflowTrigger::TurnEnd)
            })
            .collect();
        assert_eq!(matching.len(), 1);
    }

    // --- Test 19: controller_ignores_disabled_attachments ---

    #[rstest::rstest]
    fn controller_ignores_disabled_attachments() {
        let h = TestHarness::new();
        let mut session = ChatSessionState::new();
        let mut aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 3,
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
            },
            WorkflowTrigger::TurnEnd,
        );
        aw.enabled = false;
        session.core.attached_workflows.push(aw);
        let session_id = h.insert_session(session);

        let guard = h.state.read();
        let session = guard.session.get(&session_id).expect("session");
        let matching: Vec<_> = session
            .core
            .attached_workflows
            .iter()
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
            WorkflowConfig::Consensus {
                n: 3,
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
            },
            WorkflowTrigger::Manual,
        );
        session.core.attached_workflows.push(aw);
        let session_id = h.insert_session(session);

        let guard = h.state.read();
        let session = guard.session.get(&session_id).expect("session");
        let matching: Vec<_> = session
            .core
            .attached_workflows
            .iter()
            .filter(|aw| {
                matches!(
                    aw.trigger,
                    WorkflowTrigger::TurnEnd | WorkflowTrigger::TurnEndOneShot
                )
            })
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
        let mut actor = WorkflowControllerActor { ctx, state, services: h.services.clone() };

        let session = ChatSessionState::new();
        let session_id = session.session_id().clone();
        h.state.write().session.insert(session);

        actor.handle_attach_workflow(&AttachWorkflow {
            session_id: session_id.clone(),
            config: WorkflowConfig::Consensus {
                n: 3,
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
            },
            trigger: WorkflowTrigger::TurnEnd,
        });

        let guard = h.state.read();
        let session = guard.session.get(&session_id).expect("session");
        assert_eq!(session.core.attached_workflows.len(), 1);
        assert!(matches!(
            session.core.attached_workflows[0].trigger,
            WorkflowTrigger::TurnEnd
        ));
    }

    // --- Test 34: detach_workflow_command_removes_attachment ---

    #[rstest::rstest]
    fn detach_workflow_command_removes_attachment() {
        let h = TestHarness::new();
        let services = TestServices::builder().build();
        let state = h.state.clone();
        let ctx = Arc::new(DomainNodeContext::new(services, state.clone()));
        let mut actor = WorkflowControllerActor { ctx, state, services: h.services.clone() };

        let mut session = ChatSessionState::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 3,
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
            },
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
        let mut actor = WorkflowControllerActor { ctx, state, services: h.services.clone() };

        let mut session = ChatSessionState::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 3,
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
            },
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
        let actor = WorkflowControllerActor { ctx, state, services: h.services.clone() };

        let mut session = ChatSessionState::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 3,
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
            },
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
        assert!(matches!(
            session.core.attached_workflows[0].state,
            AttachedWorkflowState::Running
        ));
    }

    // --- Test 29: controller_resets_to_ready_on_cancel ---

    #[rstest::rstest]
    fn controller_resets_to_ready_on_cancel() {
        let h = TestHarness::new();
        let services = TestServices::builder().build();
        let state = h.state.clone();
        let ctx = Arc::new(DomainNodeContext::new(services, state.clone()));
        let actor = WorkflowControllerActor { ctx, state, services: h.services.clone() };

        let mut session = ChatSessionState::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 3,
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
            },
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
        assert!(matches!(
            session.core.attached_workflows[0].state,
            AttachedWorkflowState::Ready
        ));
    }

    // --- Test 30: controller_sets_failed_on_execution_error ---

    #[rstest::rstest]
    fn controller_sets_failed_on_execution_error() {
        let h = TestHarness::new();

        let services = TestServices::builder().build();

        let state = h.state.clone();
        let ctx = Arc::new(DomainNodeContext::new(services, state.clone()));
        let actor = WorkflowControllerActor { ctx, state, services: h.services.clone() };

        let mut session = ChatSessionState::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 3,
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
            },
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
        let mut actor = WorkflowControllerActor { ctx, state, services: h.services.clone() };

        let mut session = ChatSessionState::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 3,
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
            },
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
        assert!(matches!(
            session.core.attached_workflows[0].state,
            AttachedWorkflowState::Ready
        ));
    }

    // --- Test 32: controller_handles_detach_while_running ---

    #[rstest::rstest]
    fn controller_handles_detach_while_running() {
        let h = TestHarness::new();
        let services = TestServices::builder().build();
        let state = h.state.clone();
        let ctx = Arc::new(DomainNodeContext::new(services, state.clone()));
        let mut actor = WorkflowControllerActor { ctx, state, services: h.services.clone() };

        let mut session = ChatSessionState::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 3,
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
            },
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
        h.state
            .write()
            .workflow_executions
            .insert(wf_id.clone(), exec_state);

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

    // --- BeforeTurn merge strategy tests ---

    #[rstest::rstest]
    fn before_turn_sequential_queues_remaining() {
        use crate::feat::workflow::attached_workflow::{BeforeTurnMode, PromptMergeStrategy};

        let h = TestHarness::new();
        let mut session = ChatSessionState::new();

        // Create two BeforeTurn attachments.
        let aw1 = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 1,
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
            },
            WorkflowTrigger::BeforeTurn(BeforeTurnMode::AutoSend {
                strategy: PromptMergeStrategy::Replace,
            }),
        );
        let aw1_id = aw1.id.clone();
        let aw2 = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 1,
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
            },
            WorkflowTrigger::BeforeTurn(BeforeTurnMode::AutoSend {
                strategy: PromptMergeStrategy::Append,
            }),
        );
        let aw2_id = aw2.id.clone();

        session.core.attached_workflows.push(aw1);
        session.core.attached_workflows.push(aw2);
        let session_id = h.insert_session(session);

        // Simulate handle_fire_before_turn: first is fired, second is queued.
        // Clone first, then mutate.
        let aw2_clone = {
            let guard = h.state.read();
            guard
                .session
                .get(&session_id)
                .unwrap()
                .core
                .attached_workflows[1]
                .clone()
        };
        {
            let mut guard = h.state.write();
            guard.before_turn_queue.insert(
                session_id.clone(),
                vec![(
                    aw2_clone,
                    BeforeTurnMode::AutoSend {
                        strategy:
                            crate::feat::workflow::attached_workflow::PromptMergeStrategy::Append,
                    },
                )],
            );
            guard.pending_before_turn.insert(
                session_id.clone(),
                BeforeTurnMode::AutoSend {
                    strategy:
                        crate::feat::workflow::attached_workflow::PromptMergeStrategy::Replace,
                },
            );
        }

        // Verify queue has one entry.
        let guard = h.state.read();
        let queue = guard.before_turn_queue.get(&session_id).expect("queue");
        assert_eq!(queue.len(), 1);
        assert!(guard.pending_before_turn.contains_key(&session_id));
    }

    #[rstest::rstest]
    fn before_turn_queue_cleared_on_deactivate() {
        use crate::feat::workflow::attached_workflow::BeforeTurnMode;

        let h = TestHarness::new();
        let mut session = ChatSessionState::new();
        let session_id = h.insert_session(session);

        // Populate both fields.
        h.state.write().pending_before_turn.insert(
            session_id.clone(),
            BeforeTurnMode::AutoSend {
                strategy: crate::feat::workflow::attached_workflow::PromptMergeStrategy::Replace,
            },
        );
        h.state
            .write()
            .before_turn_queue
            .insert(session_id.clone(), vec![]);

        // Simulate deactivate clearing.
        {
            let mut guard = h.state.write();
            guard.pending_before_turn.clear();
            guard.before_turn_queue.clear();
        }

        let guard = h.state.read();
        assert!(guard.pending_before_turn.is_empty());
        assert!(guard.before_turn_queue.is_empty());
    }

    // --- Integration tests: BeforeTurn merge strategies ---
    //
    // These test the merge logic (Replace/Prepend/Append) by simulating
    // what apply_workflow_result does internally, without going through
    // the actor's write-lock path (which deadlocks in sync test context).
    // The real method is covered by the tokio::spawn-based tests above.

    fn merge_text(
        strategy: &crate::feat::workflow::attached_workflow::PromptMergeStrategy,
        original: &str,
        enhanced: &str,
    ) -> String {
        use crate::feat::workflow::attached_workflow::PromptMergeStrategy;
        match strategy {
            PromptMergeStrategy::Replace => enhanced.to_owned(),
            PromptMergeStrategy::Prepend => format!("{enhanced}\n{original}"),
            PromptMergeStrategy::Append => format!("{original}\n{enhanced}"),
        }
    }

    #[rstest::rstest]
    fn merge_strategy_replace_drops_original() {
        let result = merge_text(
            &crate::feat::workflow::attached_workflow::PromptMergeStrategy::Replace,
            "original",
            "enhanced",
        );
        assert_eq!(result, "enhanced");
    }

    #[rstest::rstest]
    fn merge_strategy_prepend_puts_enhanced_first() {
        let result = merge_text(
            &crate::feat::workflow::attached_workflow::PromptMergeStrategy::Prepend,
            "original",
            "enhanced",
        );
        assert_eq!(result, "enhanced\noriginal");
    }

    #[rstest::rstest]
    fn merge_strategy_append_puts_enhanced_last() {
        let result = merge_text(
            &crate::feat::workflow::attached_workflow::PromptMergeStrategy::Append,
            "original",
            "enhanced",
        );
        assert_eq!(result, "original\nenhanced");
    }

    #[rstest::rstest]
    fn merge_strategy_replace_with_empty_original() {
        let result = merge_text(
            &crate::feat::workflow::attached_workflow::PromptMergeStrategy::Replace,
            "",
            "enhanced",
        );
        assert_eq!(result, "enhanced");
    }

    #[rstest::rstest]
    fn merge_strategy_prepend_with_empty_enhanced() {
        let result = merge_text(
            &crate::feat::workflow::attached_workflow::PromptMergeStrategy::Prepend,
            "original",
            "",
        );
        assert_eq!(result, "\noriginal");
    }

    #[rstest::rstest]
    fn before_turn_queue_sequential_ordering() {
        // Verify that the before_turn_queue stores attachments in order
        // and that sequential execution would process them FIFO.
        use crate::feat::workflow::attached_workflow::{
            AttachedWorkflow, BeforeTurnMode, PromptMergeStrategy, WorkflowConfig, WorkflowTrigger,
        };

        let h = TestHarness::new();
        let mut session = ChatSessionState::new();

        let aw1 = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 1,
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
            },
            WorkflowTrigger::BeforeTurn(BeforeTurnMode::AutoSend {
                strategy: PromptMergeStrategy::Replace,
            }),
        );
        let aw2 = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 1,
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Assistant,
            },
            WorkflowTrigger::BeforeTurn(BeforeTurnMode::AutoSend {
                strategy: PromptMergeStrategy::Append,
            }),
        );
        let aw1_id = aw1.id.clone();
        let aw2_id = aw2.id.clone();

        session.core.attached_workflows.push(aw1);
        session.core.attached_workflows.push(aw2);
        let session_id = h.insert_session(session);

        // Simulate handle_fire_before_turn queueing the second attachment.
        let aw2_clone = {
            let guard = h.state.read();
            guard
                .session
                .get(&session_id)
                .unwrap()
                .core
                .attached_workflows[1]
                .clone()
        };
        {
            let mut guard = h.state.write();
            guard.before_turn_queue.insert(
                session_id.clone(),
                vec![(
                    aw2_clone,
                    BeforeTurnMode::AutoSend {
                        strategy:
                            crate::feat::workflow::attached_workflow::PromptMergeStrategy::Append,
                    },
                )],
            );
            guard.pending_before_turn.insert(
                session_id.clone(),
                BeforeTurnMode::AutoSend {
                    strategy:
                        crate::feat::workflow::attached_workflow::PromptMergeStrategy::Replace,
                },
            );
        }

        // Verify queue has one entry and pending_before_turn is set.
        let guard = h.state.read();
        let queue = guard.before_turn_queue.get(&session_id).expect("queue");
        assert_eq!(queue.len(), 1);
        assert!(guard.pending_before_turn.contains_key(&session_id));
    }

    // --- Integration test: judge_fail end-to-end ---


    #[tokio::test]
    async fn judge_fail_pushes_user_entry_to_session() {
        use std::io::Write as _;

        // Given a temp dir with the judge_fail plugin.
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let plugin_dir = temp_dir.path().join("share").join("plugins").join("judge_fail");
        std::fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        let mut f = std::fs::File::create(plugin_dir.join("init.lua")).expect("create init.lua");
        write!(
            f,
            r"return {{
                run = function(ctx)
                    ctx.push_user('judgement failed, try again')
                end
            }}"
        )
        .expect("write init.lua");

        let paths = crate::common::app_paths::AppPaths::new_in(temp_dir.path());
        let services = TestServices::builder().paths(paths).build();
        let state = State::new(AppState::default());
        let ctx = Arc::new(DomainNodeContext::new(services.clone(), state.clone()));

        // Given a session with an attached Judge workflow.
        let session_id = state.read().session.active_session_id().clone();
        let workflow_id = WorkflowId::new();
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Judge {
                prompt: String::new(),
                approval_tool: "task_complete".to_owned(),
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Silent,
                script: "judge_fail".to_owned(),
            },
            WorkflowTrigger::TurnEnd,
        );
        {
            let mut guard = state.write();
            let session = guard.session.get_mut(&session_id).expect("session");
            let mut custom_aw = aw;
            custom_aw.id = workflow_id.clone();
            session.core.attached_workflows.push(custom_aw);
        }

        let actor = WorkflowControllerActor {
            ctx,
            state: state.clone(),
            services,
        };

        // When spawning the Lua workflow.
        let handle = actor.spawn_lua_workflow(
            &WorkflowConfig::Judge {
                prompt: String::new(),
                approval_tool: "task_complete".to_owned(),
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Silent,
                script: "judge_fail".to_owned(),
            },
            &session_id,
            &workflow_id,
            &WorkflowTrigger::TurnEnd,
        );

        let result = handle.await.expect("join");

        assert!(result.is_ok(), "workflow failed: {:?}", result);

        // Then a user entry was pushed to session history.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");

        let last = session.history().last().expect("entry exists");
        match &last.kind {
            crate::feat::session::chat_entry::ChatEntryKind::User { display, .. } => {
                assert_eq!(display, "judgement failed, try again");
            }
            other => panic!("expected User entry, got {other:?}"),
        }
    }

    // --- Integration test: judge_pass end-to-end ---

    #[tokio::test]
    async fn judge_pass_pushes_system_entry_and_disables_workflow() {
        // Given a temp dir with the judge_pass plugin.
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let plugin_dir = temp_dir
            .path()
            .join("share")
            .join("plugins")
            .join("judge_pass");
        std::fs::create_dir_all(&plugin_dir).expect("create plugin dir");

        let lua_script = r#"return {
    run = function(ctx)
        ctx.push_system('judgement passed')
        ctx.turn_off()
    end
}
"#;
        let mut f = std::fs::File::create(plugin_dir.join("init.lua")).expect("create init.lua");
        use std::io::Write;
        write!(f, "{lua_script}").expect("write init.lua");

        let paths =
            crate::common::app_paths::AppPaths::new_in(temp_dir.path());
        let services = TestServices::builder().paths(paths).build();
        let state = State::new(AppState::default());
        let session_id = state.read().session.active_session_id().clone();

        let ctx = Arc::new(DomainNodeContext::new(services.clone(), state.clone()));
        let actor = WorkflowControllerActor {
            ctx,
            state: state.clone(),
            services: services.clone(),
        };

        let workflow_id = WorkflowId::new();
        {
            let mut guard = state.write();
            let session = guard.session.get_mut(&session_id).expect("session");
            let aw = AttachedWorkflow::new(
                WorkflowConfig::Judge {
                    prompt: String::new(),
                    approval_tool: "task_complete".to_owned(),
                    result_kind: crate::feat::workflow::attached_workflow::ResultKind::Silent,
                    script: "judge_pass".to_owned(),
                },
                WorkflowTrigger::TurnEnd,
            );
            let mut custom_aw = aw;
            custom_aw.id = workflow_id.clone();
            session.core.attached_workflows.push(custom_aw);
        }

        // When spawning the judge_pass workflow.
        let handle = actor.spawn_lua_workflow(
            &WorkflowConfig::Judge {
                prompt: String::new(),
                approval_tool: "task_complete".to_owned(),
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Silent,
                script: "judge_pass".to_owned(),
            },
            &session_id,
            &workflow_id,
            &WorkflowTrigger::TurnEnd,
        );

        let result = handle.await.expect("join");
        assert!(result.is_ok(), "workflow failed: {:?}", result);

        // Then a system entry was pushed to session history.
        {
            let guard = state.read();
            let session = guard.session.get(&session_id).expect("session");
            let last = session.history().last().expect("entry exists");
            match &last.kind {
                crate::feat::session::chat_entry::ChatEntryKind::System(text) => {
                    assert_eq!(text, "judgement passed");
                }
                other => panic!("expected System entry, got {other:?}"),
            }
        }

        // And the attached workflow is disabled.
        {
            let guard = state.read();
            let session = guard.session.get(&session_id).expect("session");
            let aw = session
                .core
                .attached_workflows
                .iter()
                .find(|aw| aw.id == workflow_id)
                .expect("workflow");
            assert!(!aw.enabled);
            assert!(matches!(
                aw.state,
                crate::feat::workflow::attached_workflow::AttachedWorkflowState::Completed
            ));
        }
    }

    // --- Integration test: TurnEnd trigger fires Lua path end-to-end ---

    #[tokio::test]
    async fn turn_end_trigger_fires_judge_fail_lua_workflow() {
        // Given a temp dir with the judge_fail plugin.
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let plugin_dir = temp_dir
            .path()
            .join("share")
            .join("plugins")
            .join("judge_fail");
        std::fs::create_dir_all(&plugin_dir).expect("create plugin dir");

        let lua_script = r#"return {
    run = function(ctx)
        ctx.push_user('judgement failed, try again')
    end
}"#;
        let mut f = std::fs::File::create(plugin_dir.join("init.lua")).expect("create init.lua");
        use std::io::Write;
        write!(f, "{lua_script}").expect("write init.lua");

        let paths =
            crate::common::app_paths::AppPaths::new_in(temp_dir.path());
        let services = TestServices::builder().paths(paths).build();
        let state = State::new(AppState::default());
        let session_id = state.read().session.active_session_id().clone();

        let ctx = Arc::new(DomainNodeContext::new(services.clone(), state.clone()));
        let mut actor = WorkflowControllerActor {
            ctx,
            state: state.clone(),
            services: services.clone(),
        };

        // Given an attached Judge workflow with TurnEnd trigger.
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Judge {
                prompt: String::new(),
                approval_tool: "task_complete".to_owned(),
                result_kind: crate::feat::workflow::attached_workflow::ResultKind::Silent,
                script: "judge_fail".to_owned(),
            },
            WorkflowTrigger::TurnEnd,
        );
        {
            let mut guard = state.write();
            let session = guard.session.get_mut(&session_id).expect("session");
            session.core.attached_workflows.push(aw);
        }

        // When the session phase changes to Idle (simulating TurnEnd).
        let payload = SessionPhaseChanged {
            session_id: session_id.clone(),
            old_phase: PhaseKind::Streaming,
            new_phase: PhaseKind::Idle,
        };
        actor.handle_session_phase_changed(&payload).await;

        // Then the Lua script has pushed a user entry to the session.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");
        let history = session.history();
        assert_eq!(history.len(), 1, "expected exactly one entry");
        let last = history.last().expect("entry exists");
        match &last.kind {
            crate::feat::session::chat_entry::ChatEntryKind::User { display, .. } => {
                assert_eq!(display, "judgement failed, try again");
            }
            other => panic!("expected User entry, got {other:?}"),
        }
    }
}
