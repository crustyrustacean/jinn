//! [`SidebarSection`] trait and supporting types for pluggable sidebar sections.

use crate::Intent;
use crate::common::app_state::AppState;
use ratatui::Frame;
use ratatui::layout::Rect;

/// Identifies a sidebar section. Used for focus tracking and dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SidebarSectionId {
    /// The pinned context entries section.
    Pins,
}

impl std::fmt::Display for SidebarSectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pins => write!(f, "Pins"),
        }
    }
}

impl Default for SidebarSectionId {
    fn default() -> Self {
        Self::Pins
    }
}

/// Configuration passed to a section during intent handling.
///
/// Tells the section whether other sections exist above/below it,
/// enabling boundary-aware navigation (sticky vs. unhandle).
#[derive(Debug, Clone, Copy)]
pub struct SidebarSectionConfig {
    /// Whether another section exists above this one.
    pub has_above: bool,
    /// Whether another section exists below this one.
    pub has_below: bool,
}

impl SidebarSectionConfig {
    /// Creates a config for a section that is the only one (no above, no below).
    #[must_use]
    pub const fn isolated() -> Self {
        Self {
            has_above: false,
            has_below: false,
        }
    }
}

/// Result returned by a section after handling an intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarSectionResult {
    /// The section handled the intent (e.g., moved selection).
    Handled,
    /// The section cannot move down further (at bottom of its list).
    UnhandledDown,
    /// The section cannot move up further (at top of its list).
    UnhandledUp,
}

/// Intents that the sidebar dispatches to its sections.
#[derive(Debug, Clone)]
pub enum SidebarIntent {
    /// Move selection down within the section.
    MoveDown,
    /// Move selection up within the section.
    MoveUp,
    /// A section-specific action, wrapping the app-level intent.
    Action(Intent),
}

/// A pluggable section within the sidebar.
///
/// Sections are responsible for:
/// - Rendering themselves within an allocated area
/// - Handling navigation and action intents
/// - Reporting their content height for scrolling calculations
///
/// When a section receives a `MoveDown`/`MoveUp` intent and is at its boundary,
/// it returns `UnhandledDown`/`UnhandledUp` so the sidebar can move focus to
/// the next section. The section must deselect its items when returning unhandled
/// (so the next section can take over highlighting).
pub trait SidebarSection: std::fmt::Debug + 'static {
    /// Returns the unique identifier for this section.
    fn id(&self) -> SidebarSectionId;

    /// Handle a sidebar intent.
    ///
    /// The `config` parameter tells the section whether other sections exist
    /// above/below, enabling boundary-aware behavior:
    /// - If `config.has_below` is false and at the bottom, return `UnhandledDown`
    ///   (the sidebar will try to move to the next section, or it sticks if alone).
    /// - If `config.has_above` is false and at the top, return `UnhandledUp`.
    fn handle_intent(
        &mut self,
        intent: &SidebarIntent,
        state: &mut AppState,
        config: &SidebarSectionConfig,
    ) -> SidebarSectionResult;

    /// Render the section into the given frame area.
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState);

    /// Returns the total content height in rows for the current state.
    ///
    /// Used by the sidebar for scrolling calculations.
    fn content_height(&self, state: &AppState) -> u16;
}
