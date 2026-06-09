//! Sidebar state - focus tracking.
//!
//! The focused section is now derived from the `FocusScope` variant on the
//! scope stack, so this struct is currently empty. Kept as a placeholder
//! for future sidebar-level state.

/// State for the generic sidebar UI component.
///
/// Tracks which section has keyboard focus. Scope entry/exit is managed
/// by the [`ScopeStack`](crate::common::app_state::ScopeStack) on `FrontendState`.
#[derive(Debug, Clone, Default)]
pub struct SidebarState;
