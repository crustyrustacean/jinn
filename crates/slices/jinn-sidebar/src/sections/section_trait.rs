//! [`SidebarSection`] trait and supporting types for pluggable sidebar sections.

use jinn_slices::DrawContext;

use jinn_kernel::KernelIntent;
use ratatui::Frame;
use ratatui::layout::Rect;

/// Identifies a sidebar section (shared vocabulary from `jinn-slices`;
/// the focus stack carries it in its sidebar scopes).
pub use jinn_sidebar_msg::sidebar_section_id::SidebarSectionId;

/// Result of a section navigation attempt.
///
/// Sections report `Exhausted` when they run out of entries - the sidebar
/// then decides whether to switch sections or keep the cursor where it is.
/// The section does NOT modify its cursor on exhaustion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionNavResult {
    /// Cursor moved within the section. Section already updated its own state.
    Moved,
    /// No more entries in the requested direction. Section did NOT touch
    /// its cursor - the sidebar decides what happens next.
    Exhausted,
}

/// Which end to place the cursor on when entering a section.
#[derive(Debug, Clone, Copy)]
pub enum EnterFrom {
    /// Entering from above - select the first entry.
    Top,
    /// Entering from below - select the last entry.
    Bottom,
}

/// Intents that the sidebar dispatches to its sections.
#[derive(Debug, Clone)]
pub enum SidebarIntent {
    /// Move selection down within the section.
    MoveDown,
    /// Move selection up within the section.
    MoveUp,
    /// A section-specific action, wrapping the app-level intent.
    Action(KernelIntent),
}

/// A pluggable section within the sidebar.
///
/// Sections are responsible for:
/// - Rendering themselves within an allocated area
/// - Reporting their content height for layout calculations
///
/// Navigation is handled by standalone `navigate`/`receive_cursor` functions
/// per section, orchestrated by `navigate_sidebar` in the sidebar module.
pub trait SidebarSection: std::fmt::Debug + Send + 'static {
    /// Returns the unique identifier for this section.
    fn id(&self) -> SidebarSectionId;

    /// Render the section into the given frame area.
    ///
    /// `skip_rows` is how many of the section's own leading rows are scrolled
    /// above the column. The section must drop that many lines before drawing,
    /// because a widget has no notion of a document offset — it only knows the
    /// `Rect` it was handed. Sections that render a `Paragraph` should apply
    /// `skip_rows` to its own `.scroll()`.
    fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        skip_rows: u16,
        ctx: &dyn DrawContext<jinn_kernel::common::app_state::AppState>,
    );

    /// Returns the total content height in rows for the current state.
    ///
    /// The sidebar sums these into a single document and windows it, so a
    /// section reports its full height regardless of how tall the column is.
    /// Takes `&mut self` so a section may memoize expensive layout work shared
    /// with `render` — the sidebar calls this once per section per frame,
    /// immediately before rendering that same section.
    fn content_height(
        &mut self,
        ctx: &dyn DrawContext<jinn_kernel::common::app_state::AppState>,
    ) -> u16;
}
