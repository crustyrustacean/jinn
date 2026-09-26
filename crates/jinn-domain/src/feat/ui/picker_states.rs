//! Picker state grouping and accessor trait.
//!
//! All picker-related state (picker widgets, snapshots, scroll offsets) is grouped
//! into [`PickerStates`]. The [`PickerExt`] extension trait provides accessor methods
//! on [`FrontendState`](super::FrontendState) so consumers are decoupled from the
//! internal storage layout.

use jinn_mcp_msg::McpServerEntry;
use jinn_project_msg::ProjectEntry;
use jinn_provider_selection_msg::ProviderPickerEntry;
use jinn_provider_selection_msg::endpoint::EndpointEntry;
use jinn_session_store_msg::SessionTreeEntry;
use jinn_tools_msg::TaskListTreeEntry;

/// All picker state - grouped so the picker subsystem can evolve independently.
///
/// Each picker has its own selection state and optional companion fields
/// (snapshots, scroll offsets) used during the picker's open/close lifecycle.
#[derive(Debug, Default)]
pub struct PickerStates {
    /// Session picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (session picker navigation).
    pub session_picker:
        jinn_selection_widget::TreePickerState<jinn_picker::PickerEntry<SessionTreeEntry>>,

    /// Preview pane scroll offsets for spec-driven pickers, keyed by
    /// picker id.
    pub pickers_scrolls: jinn_picker::PickerScrolls,

    /// Task list picker state - read-only zoom view of the active session's task list.
    /// OWNER: IntentHandler (populated on task list picker open).
    pub task_list_picker:
        jinn_selection_widget::TreePickerState<jinn_picker::PickerEntry<TaskListTreeEntry>>,

    pub project_picker:
        jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<ProjectEntry>>,

    /// Measured results-area row count for the currently-active picker, as
    /// written by the TUI render pre-pass each frame. Used by the picker
    /// navigation intents to keep the cursor inside the visible window.
    ///
    /// Zero before the first render of a picker; the intent layer falls back
    /// to a sane default in that case.
    /// OWNER: TUI render pre-pass (writes) / IntentHandler (reads via
    /// `active_viewport`).
    pub picker_results_viewport: u16,

    /// MCP server picker state - shows configured servers with toggle state.
    /// OWNER: IntentHandler (populated on MCP picker open).
    pub mcp_server_picker:
        jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<McpServerEntry>>,

    /// Snapshot of enabled MCP servers before picker opens - restored on ESC.
    /// OWNER: IntentHandler (set on MCP picker open, consumed on confirm/cancel).
    pub mcp_server_picker_snapshot: Option<std::collections::BTreeSet<String>>,

    /// Provider picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (navigation) / provider-selection slice's
    /// `ProviderActor` (fills items at load time through the
    /// `State::with_pickers` projection). The cell ([`jinn_provider_selection_msg::
    /// ProviderCell`]) holds the source data; this field is the
    /// render/navigation surface the picker host lends from `&AppState`.
    pub provider_picker:
        jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<ProviderPickerEntry>>,

    /// OpenRouter endpoint picker state - one row per routing upstream.
    /// OWNER: IntentHandler (navigation) / provider-selection slice's
    /// `ProviderActor` (fills items at load time through the
    /// `State::with_pickers` projection). Endpoint loading/fetched-at flags live on the
    /// provider cell.
    pub endpoint_picker:
        jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<EndpointEntry>>,
}

/// Extension trait providing typed access to picker state on [`FrontendState`](super::FrontendState).
///
/// Import this trait to access picker fields through methods instead of direct field access.
/// This decouples consumers from the internal storage layout of `FrontendState`.
pub trait PickerExt {
    /// Read-only access to the session picker state.
    fn session_picker(
        &self,
    ) -> &jinn_selection_widget::TreePickerState<jinn_picker::PickerEntry<SessionTreeEntry>>;
    /// Mutable access to the session picker state.
    fn session_picker_mut(
        &mut self,
    ) -> &mut jinn_selection_widget::TreePickerState<jinn_picker::PickerEntry<SessionTreeEntry>>;

    /// Read-only access to the enabled MCP servers snapshot.
    fn mcp_server_picker_snapshot(&self) -> &Option<std::collections::BTreeSet<String>>;
    /// Mutable access to the enabled MCP servers snapshot.
    fn mcp_server_picker_snapshot_mut(&mut self)
    -> &mut Option<std::collections::BTreeSet<String>>;
    /// Read-only access to the task list picker state.
    fn task_list_picker(
        &self,
    ) -> &jinn_selection_widget::TreePickerState<jinn_picker::PickerEntry<TaskListTreeEntry>>;
    /// Mutable access to the task list picker state.
    fn task_list_picker_mut(
        &mut self,
    ) -> &mut jinn_selection_widget::TreePickerState<jinn_picker::PickerEntry<TaskListTreeEntry>>;

    /// Read-only access to the project picker state.
    fn project_picker(
        &self,
    ) -> &jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<ProjectEntry>>;
    fn project_picker_mut(
        &mut self,
    ) -> &mut jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<ProjectEntry>>;

    /// Read-only access to the MCP server picker state.
    fn mcp_server_picker(
        &self,
    ) -> &jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<McpServerEntry>>;
    /// Mutable access to the MCP server picker state.
    fn mcp_server_picker_mut(
        &mut self,
    ) -> &mut jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<McpServerEntry>>;

    /// Read-only access to the OpenRouter endpoint picker state.
    fn endpoint_picker(
        &self,
    ) -> &jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<EndpointEntry>>;
    /// Mutable access to the OpenRouter endpoint picker state.
    fn endpoint_picker_mut(
        &mut self,
    ) -> &mut jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<EndpointEntry>>;

    fn picker_results_viewport(&self) -> u16;

    /// Updates the measured results-area row count. Called once per frame
    /// from the render pre-pass.
    fn set_picker_results_viewport(&mut self, val: u16);
}

impl PickerExt for super::frontend_state::FrontendState {
    fn session_picker(
        &self,
    ) -> &jinn_selection_widget::TreePickerState<jinn_picker::PickerEntry<SessionTreeEntry>> {
        &self.pickers.session_picker
    }

    fn session_picker_mut(
        &mut self,
    ) -> &mut jinn_selection_widget::TreePickerState<jinn_picker::PickerEntry<SessionTreeEntry>>
    {
        &mut self.pickers.session_picker
    }

    fn mcp_server_picker_snapshot(&self) -> &Option<std::collections::BTreeSet<String>> {
        &self.pickers.mcp_server_picker_snapshot
    }

    fn mcp_server_picker_snapshot_mut(
        &mut self,
    ) -> &mut Option<std::collections::BTreeSet<String>> {
        &mut self.pickers.mcp_server_picker_snapshot
    }

    fn task_list_picker(
        &self,
    ) -> &jinn_selection_widget::TreePickerState<jinn_picker::PickerEntry<TaskListTreeEntry>> {
        &self.pickers.task_list_picker
    }

    fn task_list_picker_mut(
        &mut self,
    ) -> &mut jinn_selection_widget::TreePickerState<jinn_picker::PickerEntry<TaskListTreeEntry>>
    {
        &mut self.pickers.task_list_picker
    }
    fn project_picker(
        &self,
    ) -> &jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<ProjectEntry>> {
        &self.pickers.project_picker
    }

    fn project_picker_mut(
        &mut self,
    ) -> &mut jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<ProjectEntry>> {
        &mut self.pickers.project_picker
    }

    fn mcp_server_picker(
        &self,
    ) -> &jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<McpServerEntry>> {
        &self.pickers.mcp_server_picker
    }

    fn mcp_server_picker_mut(
        &mut self,
    ) -> &mut jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<McpServerEntry>> {
        &mut self.pickers.mcp_server_picker
    }

    fn endpoint_picker(
        &self,
    ) -> &jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<EndpointEntry>> {
        &self.pickers.endpoint_picker
    }

    fn endpoint_picker_mut(
        &mut self,
    ) -> &mut jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<EndpointEntry>> {
        &mut self.pickers.endpoint_picker
    }

    fn picker_results_viewport(&self) -> u16 {
        self.pickers.picker_results_viewport
    }

    fn set_picker_results_viewport(&mut self, val: u16) {
        self.pickers.picker_results_viewport = val;
    }
}
