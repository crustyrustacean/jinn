//! Shared application state.
//!
//! [`AppState`] is the single source of truth for what the user sees and how the
//! application is currently behaving. Every component reads from and writes to this
//! shared state.
//!
//! Fields are grouped into owner-named structs (`Session`, `Context`, `Provider`,
//! `Shutdown`, `Frontend`) to make cross-boundary writes visually obvious during
//! code review. Each group struct carries `/// OWNER:` documentation on the struct
//! and on each field.

// --- Re-exports: types moved to their feature homes ---
pub use crate::common::session_map::SessionLoadGuard;
pub use crate::common::focus::{FocusScope, ScopeStack};
pub use crate::feat::ui::frontend_state::{FrontendCaches, FrontendState};
pub use crate::feat::context::assembly_state::ContextAssemblyState;
pub use crate::feat::provider::ProviderState;
pub use crate::feat::rename_session_input::state::RenameSessionInputState;
pub use crate::feat::session_lifecycle::arg_input_state::ArgInputState;
pub use crate::feat::workflow::workflow_ui_state::WorkflowUiState;

use crate::protocol::{ChatEntryId, PickerKind, PinPosition, SessionId};

use crate::common::session_map::SessionMap;
pub use crate::feat::chat_input::ChatInputBoxState;
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::ui::picker_states::PickerExt;

/// Written to exclusively by `SessionPersistenceActor` and `IntentHandler`.
/// No other actor should mutate these fields.
///
/// See [`SessionMap`] for the full API.
pub type SessionState = SessionMap;

/// A snapshot of everything the application is doing right now.
#[derive(Debug, Default)]
pub struct AppState {
    /// Session lifecycle state - owned by session-actor.
    pub session: SessionState,
    /// Context assembly state - owned by context-actor.
    pub context: ContextAssemblyState,
    /// Provider selection state - owned by provider-actor.
    pub provider: ProviderState,
    /// Frontend / UI state - owned by IntentHandler.
    pub frontend: FrontendState,
    /// Workflow execution state - owned by workflow-actor.
    pub workflow: crate::feat::workflow::workflow_state::WorkflowMap,
    /// Live executions for running attached workflows. Ephemeral (not persisted).
    /// Keyed by AttachedWorkflow.id (which IS a WorkflowId).
    /// OWNER: workflow-controller-actor.
    pub workflow_executions: std::collections::HashMap<
        crate::feat::workflow::workflow_state::WorkflowId,
        crate::feat::workflow::workflow_state::WorkflowExecutionState,
    >,
    pub active_workflow: Option<(
        crate::protocol::SessionId,
        crate::feat::workflow::workflow_state::WorkflowId,
    )>,
    pub pending_before_turn: std::collections::HashMap<
        crate::protocol::SessionId,
        crate::feat::workflow::attached_workflow::BeforeTurnMode,
    >,
    /// Queue of remaining BeforeTurn attachments for sequential execution.
    /// Key: session_id, Value: ordered list of (AttachedWorkflow, BeforeTurnMode) pairs.
    pub before_turn_queue: std::collections::HashMap<
        crate::protocol::SessionId,
        Vec<(
            crate::feat::workflow::attached_workflow::AttachedWorkflow,
            crate::feat::workflow::attached_workflow::BeforeTurnMode,
        )>,
    >,
}



impl AppState {
    /// Returns a mutable reference to the active picker's navigation interface.
    ///
    /// Returns `None` if no picker is currently active.
    /// Use for operations that work the same way on all picker types
    /// (insert char, backspace, move up/down, cursor left/right).
    pub fn active_picker_ops(&mut self) -> Option<&mut dyn jinn_selection_widget::PickerOps> {
        let kind = self.frontend.scope_stack.picker_kind().copied()?;
        match kind {
            PickerKind::Provider => Some(&mut self.provider.provider_picker),
            PickerKind::Session => Some(self.frontend.session_picker_mut()),
            PickerKind::Persona => Some(self.frontend.persona_picker_mut()),
            PickerKind::Theme => Some(self.frontend.theme_picker_mut()),

            PickerKind::SessionLifecycle => Some(self.frontend.session_lifecycle_picker_mut()),
            PickerKind::Workflow => Some(self.frontend.workflow_picker_mut()),

            PickerKind::CompactionModel => Some(self.frontend.compaction_model_picker_mut()),
            PickerKind::Tool => Some(self.frontend.tool_picker_mut()),
            PickerKind::Skill => Some(self.frontend.skill_picker_mut()),
        }
    }

    /// Read-only access to the active chat session.
    ///
    /// Infallible - `SessionMap` guarantees the active session exists.
    pub fn active_session(&self) -> &ChatSessionState {
        self.session.active_session()
    }

    /// Mutable access to the active chat session.
    ///
    /// Infallible - `SessionMap` guarantees the active session exists.
    pub fn active_session_mut(&mut self) -> &mut ChatSessionState {
        self.session.active_session_mut()
    }

    /// Read-only access to a session by ID.
    ///
    /// # Panics
    ///
    /// Panics if the given session ID does not exist.
    pub fn session(&self, id: &SessionId) -> &ChatSessionState {
        self.session.get_unchecked(id)
    }

    /// Mutable access to a session by ID.
    ///
    /// # Panics
    ///
    /// Panics if the given session ID does not exist.
    pub fn session_mut(&mut self, id: &SessionId) -> &mut ChatSessionState {
        self.session.get_unchecked_mut(id)
    }

    /// Returns mutable access to a session by ID, creating it if missing.
    ///
    /// Used by streaming handlers that receive tokens from actors
    /// which may create new session IDs not yet present in the
    /// sessions map.
    pub fn session_mut_or_create(&mut self, id: &SessionId) -> &mut ChatSessionState {
        self.session.get_or_create(id)
    }

    /// Read-only access to the active session's input box.
    ///
    /// Delegates to [`ChatSessionState::chat_input`] on the active session.
    ///
    /// # Panics
    ///
    /// Panics if the active session does not exist in the sessions map.
    pub fn active_chat_input(&self) -> &ChatInputBoxState {
        self.active_session().chat_input()
    }

    /// Mutable access to the active session's input box.
    ///
    /// Delegates to [`ChatSessionState::chat_input_mut`] on the active session.
    ///
    /// # Panics
    ///
    /// Panics if the active session does not exist in the sessions map.
    pub fn active_chat_input_mut(&mut self) -> &mut ChatInputBoxState {
        self.active_session_mut().chat_input_mut()
    }

    /// Returns pinned entry IDs sorted by position for the active session.
    ///
    /// Order: TOP entries first, then RELATIVE, then BOTTOM.
    /// Within each group, entries maintain their original history order (stable sort).
    #[must_use]
    pub fn sorted_pinned_ids(&self) -> Vec<ChatEntryId> {
        let mut pinned = self.active_session().pinned_entries();
        pinned.sort_by_key(|entry| pin_sort_key(entry.pin_position));
        pinned.iter().map(|e| e.id.clone()).collect()
    }

    /// Invalidate all theme-sensitive caches. Called when the active theme changes.
    pub fn invalidate_theme_caches(&self) {
        self.frontend.caches.invalidate_all();
    }
}

/// Returns the sort key for a pin position.
///
/// TOP = 0, RELATIVE (or None) = 1, BOTTOM = 2.
/// Used to sort pinned entries in display order.
#[must_use]
pub fn pin_sort_key(position: Option<PinPosition>) -> u8 {
    match position {
        Some(PinPosition::Top) => 0,
        Some(PinPosition::Relative) | None => 1,
        Some(PinPosition::Bottom) => 2,
    }
}
