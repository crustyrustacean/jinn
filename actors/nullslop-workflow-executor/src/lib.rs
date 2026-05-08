//! Workflow step executor actor.
//!
//! Subscribes to [`StepStarted`] and [`StreamCompleted`] events. On `StepStarted`,
//! assembles LLM context from the enriched event payload, dispatches to the LLM
//! via [`SendToLlmProvider`] commands, evaluates guards after completion, and
//! drives workflow progression by submitting [`CompleteStep`] commands back to
//! the bus. The user must explicitly approve each step before it advances.
//!
//! # Execution flow
//!
//! 1. Receive `StepStarted` (enriched with full step context).
//! 2. Assemble system prompt from instructions + completed outputs.
//! 3. Submit `SendToLlmProvider` with a unique `SessionId`.
//! 4. Wait for `StreamCompleted(Finished)` matching the step's `SessionId`.
//! 5. Evaluate guards using `DefaultGuardEvaluator<RealFileSystem, RealShell>`.
//! 6. If guards pass → submit `CompleteStep` (no auto-advance).
//! 7. If guards fail → single retry, then stop.

use std::collections::HashMap;

use nullslop_actor::{Actor, ActorContext, ActorEnvelope, SystemMessage};
use nullslop_protocol::provider::StreamCompletedReason;
use nullslop_protocol::provider::{SendToLlmProvider, StreamCompleted};
use nullslop_protocol::workflow::CompleteStep;
use nullslop_protocol::workflow::StepStarted;
use nullslop_protocol::{Command, Event, LlmMessage, SessionId};
use nullslop_workflow::template::{build_variable_map, resolve_template};
use nullslop_workflow::{
    DefaultGuardEvaluator, GuardEvaluator as _, GuardExpr, GuardFailure, GuardResult,
    RealFileSystem, RealShell, StepOutputDef,
};

/// Per-step execution state.
struct StepExecution {
    /// The step context received from `StepStarted`.
    context: StepStarted,
    /// Session ID used for the LLM dispatch (correlates `StreamCompleted` events).
    session_id: SessionId,
    /// Number of retries attempted.
    retries: u32,
}

/// Maximum retries on guard failure before giving up.
const MAX_RETRIES: u32 = 1;

/// Direct message type for the workflow executor actor.
///
/// Currently unused — the actor responds to bus commands and events.
/// Reserved for future intra-actor communication.
pub enum WorkflowExecutorDirectMsg {}

/// Workflow step executor actor.
///
/// Orchestrates step execution by dispatching LLM calls via `SendToLlmProvider`
/// and evaluating guards after completion. Submits `CompleteStep` and `AdvanceStep`
/// commands to drive workflow progression.
pub struct WorkflowExecutorActor {
    /// Active step execution (at most one at a time).
    active_step: Option<StepExecution>,
}

impl Actor for WorkflowExecutorActor {
    type Message = WorkflowExecutorDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<StepStarted>();
        ctx.subscribe_event::<StreamCompleted>();

        Self { active_step: None }
    }

    async fn handle(&mut self, msg: ActorEnvelope<WorkflowExecutorDirectMsg>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => self.handle_event(&event, ctx).await,
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::System(SystemMessage::ApplicationReady) => {
                ctx.announce_started();
            }
            ActorEnvelope::Command(_) | ActorEnvelope::Direct(_) | ActorEnvelope::Shutdown => {}
        }
    }

    async fn shutdown(self) {
        // No cleanup needed — no persistent resources.
    }
}

impl WorkflowExecutorActor {
    /// Dispatches incoming events to the appropriate handler.
    async fn handle_event(&mut self, event: &Event, ctx: &ActorContext) {
        match event {
            Event::StepStarted { payload } => {
                self.handle_step_started(payload, ctx);
            }
            Event::StreamCompleted { payload } => {
                self.handle_stream_completed(payload, ctx).await;
            }
            _ => {}
        }
    }

    /// Handles a `StepStarted` event.
    ///
    /// Assembles context and dispatches to LLM. The "pause for approval" happens
    /// after the LLM responds (via `AwaitingInput` status), not before.
    fn handle_step_started(&mut self, context: &StepStarted, ctx: &ActorContext) {
        // If there's already an active step, log and ignore.
        if self.active_step.is_some() {
            tracing::warn!(
                step_id = %context.step_id,
                "step started while another step is active, ignoring"
            );
            return;
        }

        let session_id = SessionId::new();
        let messages = assemble_step_context(context);

        tracing::info!(
            step_id = %context.step_id,
            session_id = ?session_id,
            "dispatching step to LLM"
        );

        let _ = ctx.send_command(Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: session_id.clone(),
                messages,
                provider_id: None,
            },
        });

        self.active_step = Some(StepExecution {
            context: context.clone(),
            session_id,
            retries: 0,
        });
    }

    /// Handles a `StreamCompleted` event.
    ///
    /// Matches against the active step's session ID. On `Finished`, evaluates guards
    /// and either advances or retries. On `Canceled` or other reasons, clears state.
    async fn handle_stream_completed(&mut self, completed: &StreamCompleted, ctx: &ActorContext) {
        let Some(execution) = self.active_step.take() else {
            return;
        };

        if completed.session_id != execution.session_id {
            // Not our stream — put it back.
            self.active_step = Some(execution);
            return;
        }

        match completed.reason {
            StreamCompletedReason::Finished => {
                self.on_step_finished(execution, ctx).await;
            }
            StreamCompletedReason::Canceled => {
                tracing::info!(
                    step_id = %execution.context.step_id,
                    "step stream was canceled"
                );
            }
            StreamCompletedReason::ToolUse => {
                // Tool use not yet supported in workflow executor — treat as complete.
                tracing::warn!(
                    step_id = %execution.context.step_id,
                    "tool use not yet supported in workflow steps, treating as complete"
                );
                self.on_step_finished(execution, ctx).await;
            }
        }
    }

    /// Called when an LLM stream finishes for a step.
    ///
    /// Evaluates guards. On pass, submits `CompleteStep` only (no auto-advance).
    /// The user must explicitly approve before the workflow advances.
    /// On failure, retries once.
    async fn on_step_finished(&mut self, execution: StepExecution, ctx: &ActorContext) {
        let result = evaluate_guards(&execution.context).await;

        if result.is_passed() {
            tracing::info!(
                step_id = %execution.context.step_id,
                "guards passed, completing step"
            );

            // Resolve outputs and submit CompleteStep.
            let resolved = resolve_outputs(&execution.context);
            let _ = ctx.send_command(Command::CompleteStep {
                payload: CompleteStep {
                    step_id: execution.context.step_id.clone(),
                    resolved_outputs: resolved,
                },
            });
            // No auto-advance. User must approve via 'a' key.
        } else if execution.retries < MAX_RETRIES {
            tracing::warn!(
                step_id = %execution.context.step_id,
                retries = execution.retries,
                "guards failed, retrying"
            );
            self.retry_step(execution, ctx);
        } else {
            tracing::error!(
                step_id = %execution.context.step_id,
                "guards failed after max retries, stopping step"
            );
        }
    }

    /// Retries a step by dispatching a new LLM call.
    fn retry_step(&mut self, mut execution: StepExecution, ctx: &ActorContext) {
        execution.retries += 1;
        execution.session_id = SessionId::new();
        let messages = assemble_step_context(&execution.context);

        tracing::info!(
            step_id = %execution.context.step_id,
            session_id = ?execution.session_id,
            retry = execution.retries,
            "retrying step"
        );

        let _ = ctx.send_command(Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: execution.session_id.clone(),
                messages,
                provider_id: None,
            },
        });

        self.active_step = Some(execution);
    }
}

// ---------------------------------------------------------------------------
// Context assembly
// ---------------------------------------------------------------------------

/// Assembles the LLM message list for a step.
///
/// Builds a system prompt from step instructions and completed output context,
/// plus a user message to trigger execution.
fn assemble_step_context(context: &StepStarted) -> Vec<LlmMessage> {
    let mut system = String::new();
    system.push_str("You are executing a step in a structured workflow.\n\n");
    let _ = std::fmt::Write::write_fmt(
        &mut system,
        format_args!("## Step: {}\n\n", context.step_title),
    );
    system.push_str(&context.instructions);

    // Add completed outputs as context.
    if !context.completed_outputs.is_empty() {
        system.push_str("\n\n## Completed Step Outputs\n\n");
        for (step_id, outputs) in &context.completed_outputs {
            let _ = std::fmt::Write::write_fmt(&mut system, format_args!("### Step: {step_id}\n"));
            for (label, value) in outputs {
                let _ =
                    std::fmt::Write::write_fmt(&mut system, format_args!("- {label}: {value}\n"));
            }
        }
    }

    vec![
        LlmMessage::System { content: system },
        LlmMessage::User {
            content: "Execute this step now.".to_owned(),
        },
    ]
}

// ---------------------------------------------------------------------------
// Guard evaluation
// ---------------------------------------------------------------------------

/// Evaluates the step's guards using `DefaultGuardEvaluator<RealFileSystem, RealShell>`.
///
/// Runs guard evaluation on a blocking thread to avoid stalling the async actor loop.
/// If guards are `None`, returns `Passed` immediately without blocking.
async fn evaluate_guards(context: &StepStarted) -> GuardResult {
    if context.guards == GuardExpr::None {
        return GuardResult::Passed;
    }

    let globals = context.globals.clone();
    let step_outputs: Vec<(String, String)> = context
        .completed_outputs
        .values()
        .flat_map(|m| m.iter())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let guards = context.guards.clone();
    let stored_hashes = context.stored_hashes.clone();

    tokio::task::spawn_blocking(move || -> GuardResult {
        let variables = build_variable_map(&globals, &step_outputs);
        let evaluator = DefaultGuardEvaluator::new(RealFileSystem, RealShell);
        evaluator.evaluate(&guards, &variables, &stored_hashes)
    })
    .await
    .unwrap_or_else(|e| {
        tracing::error!(err = ?e, "guard evaluation task panicked");
        GuardResult::Failed(vec![GuardFailure {
            reason: "guard evaluation task panicked".to_owned(),
        }])
    })
}

// ---------------------------------------------------------------------------
// Output resolution
// ---------------------------------------------------------------------------

/// Resolves output template values from the step's context.
///
/// Builds a variable map from globals and completed step outputs, then resolves
/// each output descriptor's template.
fn resolve_outputs(context: &StepStarted) -> HashMap<String, String> {
    let step_outputs: Vec<(String, String)> = context
        .completed_outputs
        .values()
        .flat_map(|m| m.iter())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let variables = build_variable_map(&context.globals, &step_outputs);

    context
        .outputs
        .iter()
        .map(|output| match output {
            StepOutputDef::File { label, path } => {
                (label.clone(), resolve_template(path, &variables))
            }
            StepOutputDef::Summary { label, value } => {
                (label.clone(), resolve_template(value, &variables))
            }
            StepOutputDef::Artifact { label, description } => {
                (label.clone(), resolve_template(description, &variables))
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use nullslop_actor::MessageSink;
    use nullslop_protocol::EventMsg as _;
    use nullslop_workflow::{GuardExpr, ModelHint};

    use super::*;

    /// A message sink that records commands and events for test assertions.
    struct RecordingSink {
        commands: Mutex<Vec<Command>>,
        events: Mutex<Vec<Event>>,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self {
                commands: Mutex::new(Vec::new()),
                events: Mutex::new(Vec::new()),
            }
        }

        fn commands(&self) -> Vec<Command> {
            self.commands.lock().unwrap().clone()
        }

        #[expect(dead_code, reason = "test utility")]
        fn take_commands(&self) -> Vec<Command> {
            let mut guard = self.commands.lock().unwrap();
            std::mem::take(&mut guard)
        }

        fn clear(&self) {
            self.commands.lock().unwrap().clear();
            self.events.lock().unwrap().clear();
        }
    }

    impl MessageSink for RecordingSink {
        #[expect(clippy::unwrap_in_result, reason = "test code")]
        fn send_command(&self, command: Command) -> nullslop_actor::SendResult {
            self.commands.lock().unwrap().push(command);
            Ok(())
        }

        #[expect(clippy::unwrap_in_result, reason = "test code")]
        fn send_event(&self, event: Event) -> nullslop_actor::SendResult {
            self.events.lock().unwrap().push(event);
            Ok(())
        }
    }

    /// Creates a test context backed by a recording sink.
    fn test_context(sink: &Arc<RecordingSink>) -> ActorContext {
        ActorContext::new("test-workflow-executor", sink.clone())
    }

    /// Creates a minimal `StepStarted` event for testing.
    fn make_step_started(overrides: Option<&StepStartedOverrides<'_>>) -> StepStarted {
        let defaults = StepStartedOverrides::default();
        let o = overrides.unwrap_or(&defaults);
        StepStarted {
            step_id: o.step_id.to_owned(),
            step_title: o.step_title.to_owned(),
            instructions: o.instructions.to_owned(),
            model_hint: ModelHint::Small,
            model_overrides: HashMap::new(),
            requires_user_input: o.requires_user_input,
            checkpoint: o.checkpoint,
            guards: GuardExpr::None,
            outputs: vec![],
            completed_outputs: HashMap::new(),
            globals: HashMap::new(),
            stored_hashes: HashMap::new(),
        }
    }

    struct StepStartedOverrides<'a> {
        step_id: &'a str,
        step_title: &'a str,
        instructions: &'a str,
        requires_user_input: bool,
        checkpoint: bool,
    }

    impl Default for StepStartedOverrides<'_> {
        fn default() -> Self {
            Self {
                step_id: "step-0",
                step_title: "Test Step",
                instructions: "Do the thing",
                requires_user_input: false,
                checkpoint: false,
            }
        }
    }

    // --- Activation tests ---

    #[test]
    fn activate_subscribes_to_events() {
        // Given a fresh actor context.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);

        // When activating the actor.
        let _actor = WorkflowExecutorActor::activate(&mut ctx);

        // Then the context accumulated subscriptions for StepStarted and StreamCompleted.
        let (events, _commands) = ctx.take_registrations();
        assert!(events.contains(&StepStarted::TYPE_NAME.to_owned()));
        assert!(events.contains(&StreamCompleted::TYPE_NAME.to_owned()));
    }

    // --- StepStarted always starts stream ---

    #[tokio::test]
    async fn step_started_always_starts_stream() {
        // Given an active executor.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let mut actor = WorkflowExecutorActor::activate(&mut ctx);

        // When receiving a StepStarted (no requires_user_input check).
        let started = make_step_started(None);
        actor
            .handle_event(
                &Event::StepStarted {
                    payload: Box::new(started),
                },
                &ctx,
            )
            .await;

        // Then a SendToLlmProvider command was submitted.
        let commands = sink.commands();
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            &commands[0],
            Command::SendToLlmProvider { payload } if payload.messages.len() == 2
        ));
        assert!(actor.active_step.is_some());
    }

    // --- Stream completion evaluates guards and completes (no auto-advance) ---

    #[tokio::test]
    async fn stream_completion_completes_step_without_advancing() {
        // Given an executor with an active step.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let mut actor = WorkflowExecutorActor::activate(&mut ctx);

        let started = make_step_started(None);
        let step_id = started.step_id.clone();
        actor
            .handle_event(
                &Event::StepStarted {
                    payload: Box::new(started.clone()),
                },
                &ctx,
            )
            .await;
        let session_id = actor
            .active_step
            .as_ref()
            .expect("active step")
            .session_id
            .clone();
        sink.clear();

        // When receiving StreamCompleted with Finished reason.
        let completed = StreamCompleted {
            session_id,
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("done".to_owned()),
            tool_calls: None,
        };
        actor
            .handle_event(&Event::StreamCompleted { payload: completed }, &ctx)
            .await;

        // Then CompleteStep was submitted but NOT AdvanceStep.
        let commands = sink.commands();
        let has_complete = commands.iter().any(|c| {
            matches!(
                c,
                Command::CompleteStep { payload } if payload.step_id == step_id
            )
        });
        let has_advance = commands.iter().any(|c| matches!(c, Command::AdvanceStep));
        assert!(has_complete, "expected CompleteStep command");
        assert!(!has_advance, "should not auto-advance");
        assert!(actor.active_step.is_none());
    }

    // --- All steps pause after completion (checkpoint no longer special) ---

    #[tokio::test]
    async fn checkpoint_step_completes_without_advancing() {
        // Given an executor with a checkpoint step.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let mut actor = WorkflowExecutorActor::activate(&mut ctx);

        let started = make_step_started(Some(&StepStartedOverrides {
            checkpoint: true,
            ..StepStartedOverrides::default()
        }));
        actor
            .handle_event(
                &Event::StepStarted {
                    payload: Box::new(started.clone()),
                },
                &ctx,
            )
            .await;
        let session_id = actor
            .active_step
            .as_ref()
            .expect("active step")
            .session_id
            .clone();
        sink.clear();

        // When receiving StreamCompleted with Finished reason.
        let completed = StreamCompleted {
            session_id,
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("done".to_owned()),
            tool_calls: None,
        };
        actor
            .handle_event(&Event::StreamCompleted { payload: completed }, &ctx)
            .await;

        // Then CompleteStep was submitted but NOT AdvanceStep (same as non-checkpoint).
        let commands = sink.commands();
        let has_complete = commands.iter().any(|c| {
            matches!(
                c,
                Command::CompleteStep { payload } if payload.step_id == "step-0"
            )
        });
        let has_advance = commands.iter().any(|c| matches!(c, Command::AdvanceStep));
        assert!(has_complete, "expected CompleteStep command");
        assert!(!has_advance, "should not auto-advance");
    }

    // --- Context assembly tests ---

    #[test]
    fn assembly_produces_system_message() {
        // Given a StepStarted with instructions.
        let started = StepStarted {
            step_id: "step-0".to_owned(),
            step_title: "Create Directory".to_owned(),
            instructions: "Ask the user for the directory name.".to_owned(),
            model_hint: ModelHint::Small,
            model_overrides: HashMap::new(),
            requires_user_input: false,
            checkpoint: false,
            guards: GuardExpr::None,
            outputs: vec![],
            completed_outputs: HashMap::new(),
            globals: HashMap::new(),
            stored_hashes: HashMap::new(),
        };

        // When assembling context.
        let messages = assemble_step_context(&started);

        // Then a system message is produced.
        let system_content = match &messages[0] {
            LlmMessage::System { content } => content.clone(),
            _ => panic!("expected system message"),
        };
        assert!(system_content.contains("Create Directory"));
        assert!(system_content.contains("Ask the user for the directory name"));
    }

    #[test]
    fn assembly_produces_user_message() {
        // Given a StepStarted with instructions.
        let started = StepStarted {
            step_id: "step-0".to_owned(),
            step_title: "Create Directory".to_owned(),
            instructions: "Ask the user for the directory name.".to_owned(),
            model_hint: ModelHint::Small,
            model_overrides: HashMap::new(),
            requires_user_input: false,
            checkpoint: false,
            guards: GuardExpr::None,
            outputs: vec![],
            completed_outputs: HashMap::new(),
            globals: HashMap::new(),
            stored_hashes: HashMap::new(),
        };

        // When assembling context.
        let messages = assemble_step_context(&started);

        // Then a user message is produced.
        assert_eq!(messages.len(), 2);
        let user_content = match &messages[1] {
            LlmMessage::User { content } => content.clone(),
            _ => panic!("expected user message"),
        };
        assert_eq!(user_content, "Execute this step now.");
    }

    #[test]
    fn assemble_step_context_includes_completed_outputs() {
        // Given a StepStarted with completed outputs from a previous step.
        let started = StepStarted {
            step_id: "step-1".to_owned(),
            step_title: "Process".to_owned(),
            instructions: "Use the directory.".to_owned(),
            model_hint: ModelHint::Small,
            model_overrides: HashMap::new(),
            requires_user_input: false,
            checkpoint: false,
            guards: GuardExpr::None,
            outputs: vec![],
            completed_outputs: HashMap::from([(
                "step-0".to_owned(),
                HashMap::from([("dir".to_owned(), "/tmp/test".to_owned())]),
            )]),
            globals: HashMap::new(),
            stored_hashes: HashMap::new(),
        };

        // When assembling context.
        let messages = assemble_step_context(&started);

        // Then the system prompt includes completed outputs.
        let system_content = match &messages[0] {
            LlmMessage::System { content } => content.clone(),
            _ => panic!("expected system message"),
        };
        assert!(system_content.contains("Completed Step Outputs"));
        assert!(system_content.contains("step-0"));
        assert!(system_content.contains("/tmp/test"));
    }

    // --- Output resolution tests ---

    #[test]
    fn resolve_outputs_handles_summary_output() {
        // Given a StepStarted with a summary output.
        let started = StepStarted {
            step_id: "step-0".to_owned(),
            step_title: "Test".to_owned(),
            instructions: "Test".to_owned(),
            model_hint: ModelHint::Small,
            model_overrides: HashMap::new(),
            requires_user_input: false,
            checkpoint: false,
            guards: GuardExpr::None,
            outputs: vec![nullslop_workflow::StepOutputDef::Summary {
                label: "status".to_owned(),
                value: "All done".to_owned(),
            }],
            completed_outputs: HashMap::new(),
            globals: HashMap::new(),
            stored_hashes: HashMap::new(),
        };

        // When resolving outputs.
        let resolved = resolve_outputs(&started);

        // Then the summary output is resolved.
        assert_eq!(resolved.get("status"), Some(&"All done".to_owned()));
    }

    #[test]
    fn resolve_outputs_resolves_template_variables() {
        // Given a StepStarted with template variables in outputs.
        let started = StepStarted {
            step_id: "step-0".to_owned(),
            step_title: "Test".to_owned(),
            instructions: "Test".to_owned(),
            model_hint: ModelHint::Small,
            model_overrides: HashMap::new(),
            requires_user_input: false,
            checkpoint: false,
            guards: GuardExpr::None,
            outputs: vec![nullslop_workflow::StepOutputDef::Summary {
                label: "path".to_owned(),
                value: "{{base_dir}}/output".to_owned(),
            }],
            completed_outputs: HashMap::new(),
            globals: HashMap::from([("base_dir".to_owned(), "/tmp/work".to_owned())]),
            stored_hashes: HashMap::new(),
        };

        // When resolving outputs.
        let resolved = resolve_outputs(&started);

        // Then the template variable is resolved.
        assert_eq!(resolved.get("path"), Some(&"/tmp/work/output".to_owned()));
    }

    // --- Guard evaluation tests ---

    #[tokio::test]
    async fn evaluate_guards_returns_passed_for_none() {
        // Given a StepStarted with no guards.
        let started = make_step_started(None);

        // When evaluating guards.
        let result = evaluate_guards(&started).await;

        // Then the result is Passed.
        assert!(result.is_passed());
    }

    // --- Ignore non-matching StreamCompleted ---

    #[tokio::test]
    async fn ignores_non_matching_stream_completed() {
        // Given an executor with an active step.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let mut actor = WorkflowExecutorActor::activate(&mut ctx);

        let started = make_step_started(None);
        actor
            .handle_event(
                &Event::StepStarted {
                    payload: Box::new(started),
                },
                &ctx,
            )
            .await;
        sink.clear();

        // When receiving StreamCompleted with a different session ID.
        let completed = StreamCompleted {
            session_id: SessionId::new(), // different session
            reason: StreamCompletedReason::Finished,
            assistant_content: None,
            tool_calls: None,
        };
        actor
            .handle_event(&Event::StreamCompleted { payload: completed }, &ctx)
            .await;

        // Then no commands were submitted and the active step is preserved.
        assert!(sink.commands().is_empty());
        assert!(actor.active_step.is_some());
    }
}
