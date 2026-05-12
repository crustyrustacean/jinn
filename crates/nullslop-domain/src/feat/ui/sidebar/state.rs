//! Sidebar state — focus tracking and origin scope for smart return.

use super::section_trait::SidebarSectionId;

/// The scope the user was in before entering the sidebar.
/// Used to restore the correct scope on `<c-h>` / `<esc>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarOriginScope {
    /// User was in Normal mode.
    #[default]
    Normal,
    /// User was in Input mode.
    Input,
}

/// State for the generic sidebar UI component.
///
/// Tracks which section has keyboard focus and where the user came from
/// (for smart scope return on exit).
#[derive(Debug, Clone, Default)]
pub struct SidebarState {
    /// Which section currently has keyboard focus.
    pub focused_section: SidebarSectionId,
    /// The scope the user was in before entering the sidebar.
    /// `None` means the user hasn't entered the sidebar yet in this session.
    pub origin_scope: Option<SidebarOriginScope>,
}
