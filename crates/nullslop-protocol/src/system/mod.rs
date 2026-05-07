//! System domain: application-level commands, events, and built-in actor commands.

mod command;
mod event;

pub use command::{
    DashboardSelectDown, DashboardSelectFirst, DashboardSelectLast, DashboardSelectUp, EditInput,
    MouseScrollDown, MouseScrollUp, OpenPicker, Quit, ScrollDown, ScrollLineDown, ScrollLineUp,
    ScrollToBottom, ScrollToTop, ScrollUp, SetMode, ToggleKeymapScopeFilter, ToggleWhichKey,
    WorkflowApproveStep, WorkflowFocusChat, WorkflowFocusWorkflow, WorkflowRestartStep,
    WorkflowSelectDown, WorkflowSelectFirst, WorkflowSelectLast, WorkflowSelectUp,
    WorkflowToggleDetail, WorkflowTogglePane,
};
pub use event::{KeyDown, KeyUp, ModeChanged};
