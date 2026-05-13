//! Sidebar state — focus tracking for the sidebar panel.

use super::section_trait::SidebarSectionId;

/// State for the generic sidebar UI component.
///
/// Tracks which section has keyboard focus. Scope entry/exit is managed
/// by the [`ScopeStack`](crate::common::app_state::ScopeStack) on `FrontendState`.
#[derive(Debug, Clone, Default)]
pub struct SidebarState {
    /// Which section currently has keyboard focus.
    pub focused_section: SidebarSectionId,
}
