# Workflow System Reset — Handoff Document

## Problem

The workflow system failed across 6 attempts because execution was incorrectly modeled as an async actor concern. This created three copies of state (`AppState.workflow`, `WorkflowExecutorActor.pending/active`, `ToolOrchestratorActor.workflow_cache`) that had to stay in sync. The executor actor held ephemeral state that couldn't survive restarts, orphaned sessions by dispatching to internal LLM calls, and reported success without actually executing steps.

## The right architecture

Workflows are functions. Each step is an isolated function with input and output. Execution should be a **sync command on the bus**, handled by `WorkflowHandler`. Steps spawn **child sessions** (with a new `parent_session` field linking back). When a child session's stream completes, the system collects the output and submits `CompleteStep` to the parent's workflow. `WorkflowState` in `AppState` is the single source of truth — no shadows, no caches. Save/load works automatically because it already serializes as a session blob.

## KEEP — `nullslop-workflow` crate (entire crate, as-is)

This is the pure domain library. It defines what a workflow is, where it is in its lifecycle, and how to verify step completion. No execution opinions.

| Type | Why it's correct |
|------|-----------------|
| `WorkflowDef` | Name, description, steps, globals, model_overrides. The workflow shape. |
| `StepDef` | id, title, instructions, model_hint, requires_user_input, tools, outputs, guards, depends_on. A step's contract. |
| `ModelHint` | small/medium/large capability hint for model selection. |
| `StepOutputDef` | file/summary/artifact with `{{var}}` template support. |
| `WorkflowBuilder` | Incremental construction with validation. Required for LLM-driven workflow creation. |
| `WorkflowState` | Vec+cursor state machine. `start()`, `complete_current()`, `advance()`, `fail_current()` are the right API. Single source of truth. |
| `StepState` / `StepStatus` | Per-step runtime state: pending/running/completed/failed + resolved_outputs. |
| `GuardExpr` / `GuardPredicate` | Composable guard DSL (All/Any/Not/Predicate). file_exists, dir_exists, file_hash_matches, command_succeeds, output_matches, value_set. Testable via `GuardFileSystem` / `GuardShell` trait abstractions. |
| Template resolution (`resolve_template`, `build_variable_map`) | Used by guards and outputs. |
| Hash utility | Used by file_hash_matches guard. |

## REMOVE — Everything else workflow-related

### Crates to delete entirely

| Crate | Why remove |
|-------|-----------|
| `actors/nullslop-workflow-executor/` | This was the core mistake. An async actor that held ephemeral pending/active state, manually correlated `StreamCompleted` by session ID, couldn't survive restarts. Step execution belongs in the sync `WorkflowHandler`, not an actor. |
| `crates/nullslop-workflow-store/` | Persisted workflow definitions to `~/.config/nullslop/workflows/`. The new approach may need persistence but the store's API and location should be decided by the new design. |

### Code to delete from `nullslop-tool-orchestrator`

| What | Why |
|------|-----|
| `CachedWorkflow`, `CachedStep`, `CachedStepStatus` | Third copy of workflow state. Unnecessary shadow. |
| All cache event handlers (`handle_step_started_event`, `handle_step_completed_event`, `handle_step_execution_completed_event`, `handle_workflow_completed_event`) | Maintaining the cache. |
| All 14 workflow tool implementations (`handle_workflow_create`, `handle_workflow_add_step`, `handle_workflow_commit`, `handle_workflow_run_step`, `workflow_status`, `workflow_load`, `workflow_list`, etc.) | These tools exposed workflow construction/execution to the LLM but routed through the wrong architecture. The new design will need LLM-facing tools but they should drive the bus, not maintain their own state. |
| `workflow_tool_definitions()` + `execution_tool_definitions()` | Tool schema definitions for the above. |
| `workflow_builder` and `workflow_cache` and `workflow_store` fields on the actor | Ephemeral state that doesn't belong on the tool orchestrator. |
| `nullslop-workflow` and `nullslop-workflow-store` deps from `Cargo.toml` | No longer needed by this crate. |

### Code to delete from `nullslop-component`

| What | Why |
|------|-----|
| `src/workflow/` directory (handler.rs, mod.rs) | The `WorkflowHandler` was correct in structure but referenced deleted protocol types. The new implementation will follow the same pattern. |
| `src/workflow_panel/` directory (element.rs, handler.rs, mod.rs, state.rs) | UI panel for workflow progress. Will be rebuilt in the new design. |
| Workflow registrations in `src/lib.rs` | Wiring for the above. |
| `workflow` field and methods on `ChatSessionState` (`src/chat_session/state.rs`) | `workflow: Option<WorkflowState>` and its accessors. The new design will need this field back but it should be added fresh. |
| `workflow_panel` field on `AppState` (`src/app_state.rs`) | Panel state. Will be rebuilt. |
| Workflow blob serialization in `src/provider/request_handler.rs` | The `if let Some(workflow) = session.workflow()` block in `emit_save_requested`. |
| Workflow blob deserialization in `src/session_picker/handler.rs` | The `BLOB_WORKFLOW_STATE` block in `on_session_load_completed`. |

### Code to delete from `nullslop-protocol`

| What | Why |
|------|-----|
| `src/workflow/` directory (mod.rs, command.rs, event.rs) | All workflow-specific commands (`LoadWorkflow`, `CompleteStep`, `AbortWorkflow`, `ExecuteWorkflowStep`, `FailStep`) and events (`WorkflowLoaded`, `StepStarted`, `StepCompleted`, `StepExecutionCompleted`, `WorkflowCompleted`). The new design will define its own protocol types. |
| Workflow command variants in `src/command.rs` | The `Command` enum variants referencing workflow types. |
| Workflow event variants in `src/event.rs` | The `Event` enum variants referencing workflow types. |
| Workflow-related system commands in `src/system/command.rs` | UI keybinding commands for workflow panel navigation. |

### Code to delete from `nullslop-tui`

| What | Why |
|------|-----|
| Workflow keybindings in `src/keymap.rs` | Key bindings for workflow panel interaction. |
| Workflow panel rendering in `src/render.rs` | Panel layout code. |
| Workflow scope in `src/scope.rs` | Focus scope for the workflow panel. |
| Workflow panel references in `src/app.rs` | Panel state management. |
| Workflow key handling in `src/run.rs` | Key event routing to workflow panel. |

### Code to delete from other locations

| What | Why |
|------|-----|
| Executor actor spawning in `src/app.rs` | `WorkflowExecutorActor::activate` + `spawn_actor` call. |
| Workflow-related routing in `nullslop-component-core/src/bus.rs` | Command/event dispatch branches for deleted types. |
| Workflow blob handling in `nullslop-session-actor/src/lib.rs` | If it references workflow-specific blob keys. |
| Workflow references in `src/session_conversion.rs` | Conversion helpers for workflow state. |
| 3 workspace entries in root `Cargo.toml` | `nullslop-workflow-executor`, `nullslop-workflow-store`, and the `nullslop-workflow` dependency from the main binary (keep the workspace dep since `nullslop-workflow` crate is retained). |

## What remains after cleanup

- `nullslop-workflow` crate with all domain types intact
- A clean slate to implement: `ExecuteWorkflowStep` as a real sync handler, `parent_session` on sessions, child session spawning for step execution, `CompleteStep` wired to child session completion
- The builder pattern ready for LLM-driven workflow construction via new tools on the bus
