//! System commands.

use serde::{Deserialize, Serialize};

use crate::CommandMsg;
use crate::Mode;
use crate::PickerKind;

/// Set the application interaction mode.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct SetMode {
    /// The mode to switch to.
    pub mode: Mode,
}

/// Quit the application.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct Quit;

/// Open an external editor for the input buffer.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct EditInput;

/// Toggle the which-key popup.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct ToggleWhichKey;

/// Scroll the chat log up (toward older messages).
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct ScrollUp;

/// Scroll the chat log down (toward newer messages).
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct ScrollDown;

/// Scroll the chat log up by a small amount (mouse wheel).
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct MouseScrollUp;

/// Scroll the chat log down by a small amount (mouse wheel).
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct MouseScrollDown;

/// Scroll the chat log up by one line.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct ScrollLineUp;

/// Scroll the chat log down by one line.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct ScrollLineDown;

/// Scroll the chat log to the very top.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct ScrollToTop;

/// Scroll the chat log to the very bottom.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct ScrollToBottom;

/// Move the dashboard selection down one entry.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct DashboardSelectDown;

/// Move the dashboard selection up one entry.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct DashboardSelectUp;

/// Move the dashboard selection to the first entry.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct DashboardSelectFirst;

/// Move the dashboard selection to the last entry.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct DashboardSelectLast;

/// Open a picker of the specified kind.
///
/// Sets the active picker kind, loads entries, resets the picker state,
/// and enters Picker mode.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct OpenPicker {
    /// Which picker to open.
    pub kind: PickerKind,
}

/// Toggle the keymap picker between current-scope and all-scopes view.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct ToggleKeymapScopeFilter;

/// Move the workflow panel selection down one step.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow_panel")]
pub struct WorkflowSelectDown;

/// Move the workflow panel selection up one step.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow_panel")]
pub struct WorkflowSelectUp;

/// Move the workflow panel selection to the first step.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow_panel")]
pub struct WorkflowSelectFirst;

/// Move the workflow panel selection to the last step.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow_panel")]
pub struct WorkflowSelectLast;

/// Restart the currently selected workflow step.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow_panel")]
pub struct WorkflowRestartStep;

/// Approve the currently active workflow step.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow_panel")]
pub struct WorkflowApproveStep;

/// Toggle the workflow step detail view.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow_panel")]
pub struct WorkflowToggleDetail;

/// Toggles the workflow sidebar pane visibility in the chat tab.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow_panel")]
pub struct WorkflowTogglePane;

/// Focuses the chat pane (left side) in the chat tab.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow_panel")]
pub struct WorkflowFocusChat;

/// Focuses the workflow pane (right side) in the chat tab.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("workflow_panel")]
pub struct WorkflowFocusWorkflow;

/// Toggle the pinned context panel visibility.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct PinnedPanelToggle;

/// Open the pinned context panel.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct PinnedPanelOpen;

/// Close the pinned context panel.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct PinnedPanelClose;

/// Move the pinned panel selection down one entry.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct PinnedPanelSelectDown;

/// Move the pinned panel selection up one entry.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct PinnedPanelSelectUp;

/// Unpin the currently selected pinned entry from the pinned panel.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct PinnedPanelUnpin;

/// Pin the currently selected chat entry (from chat entry selection).
///
/// TuiApp-level command — `route_command()` reads the selected entry ID
/// from state, constructs a `PinChatEntry`, and submits it to the bus.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct ChatEntryPinSelected;

/// Escape key in Normal mode: cancel selection and close pinned panel.
///
/// TuiApp-level command — `route_command()` cancels entry selection
/// (submitting `ChatEntrySelectCancel` with the real session ID) and
/// closes the pinned panel if visible.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("system")]
pub struct NormalEscape;
