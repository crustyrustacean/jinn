//! System domain: application-level commands, events, and built-in actor commands.

mod command;
mod event;

pub use command::{
    ChatEntryPinSelected, DashboardSelectDown, DashboardSelectFirst, DashboardSelectLast,
    DashboardSelectUp, EditInput, MouseScrollDown, MouseScrollUp, NormalEscape, OpenPicker,
    PinnedPanelClose, PinnedPanelOpen, PinnedPanelPinBottom, PinnedPanelPinCycle,
    PinnedPanelPinRelative, PinnedPanelPinTop, PinnedPanelSelectDown, PinnedPanelSelectUp,
    PinnedPanelToggle, PinnedPanelUnpin, Quit, ScrollDown, ScrollLineDown, ScrollLineUp,
    ScrollToBottom, ScrollToTop, ScrollUp, SetMode, ToggleKeymapScopeFilter, ToggleWhichKey,
    WorkflowApproveStep, WorkflowFocusChat, WorkflowFocusWorkflow, WorkflowRestartStep,
    WorkflowSelectDown, WorkflowSelectFirst, WorkflowSelectLast, WorkflowSelectUp,
    WorkflowToggleDetail, WorkflowTogglePane,
};
pub use event::{KeyDown, KeyUp, ModeChanged};
