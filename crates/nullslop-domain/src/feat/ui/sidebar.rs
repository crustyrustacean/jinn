//! Generic pluggable sidebar — hosts multiple sections with hybrid navigation.
//!
//! The sidebar is a persistent panel on the right side of the chat tab.
//! It contains pluggable sections (e.g., pinned entries, future features)
//! that register via the [`SidebarSection`] trait.
//!
//! Navigation uses a hybrid model: `j`/`k` moves within the focused section,
//! and sections can signal "unhandled" to let the sidebar move focus to the
//! next/previous section.

pub mod persona_section;
pub mod pins;
pub mod section_trait;
pub mod sidebar;
pub mod state;

pub use section_trait::{
    SidebarIntent, SidebarSection, SidebarSectionConfig, SidebarSectionId, SidebarSectionResult,
};
pub use sidebar::Sidebar;
pub use state::SidebarState;

/// Registers all built-in sidebar sections into the given sidebar.
pub fn register_sections(sidebar: &mut Sidebar) {
    sidebar.register(Box::new(persona_section::PersonaSection));
    sidebar.register(Box::new(pins::PinsSection));
}
