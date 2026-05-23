//! Status indicator symbols and colors.
//!
//! Maps [`NodeStatus`](nullslop_workflow::engine::NodeStatus) to visual
//! indicators: empty circle for pending, filled colored circles for
//! terminal states, and braille spinner frames for running.

use nullslop_workflow::engine::NodeStatus;
use ratatui::style::Color;

/// Braille spinner animation frames for running nodes.
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Returns the symbol for a node status indicator.
///
/// - `Pending` → `○` (empty circle)
/// - `Running` → spinner frame indexed by `tick`
/// - `Completed`/`Failed`/`Skipped` → `●` (filled circle)
#[must_use]
pub fn status_symbol(status: NodeStatus, tick: u8) -> &'static str {
    match status {
        NodeStatus::Pending => "○",
        NodeStatus::Running =>
        {
            #[expect(clippy::indexing_slicing, reason = "modulo ensures valid index")]
            SPINNER_FRAMES[(tick as usize) % SPINNER_FRAMES.len()]
        }
        NodeStatus::Completed | NodeStatus::Failed | NodeStatus::Skipped => "●",
    }
}

/// Returns the color for a node status indicator.
///
/// - `Pending` → [`Color::DarkGray`]
/// - `Running` → [`Color::Cyan`]
/// - `Completed` → [`Color::Green`]
/// - `Failed` → [`Color::Red`]
/// - `Skipped` → [`Color::Yellow`]
#[must_use]
pub fn status_color(status: NodeStatus) -> Color {
    match status {
        NodeStatus::Pending => Color::DarkGray,
        NodeStatus::Running => Color::Cyan,
        NodeStatus::Completed => Color::Green,
        NodeStatus::Failed => Color::Red,
        NodeStatus::Skipped => Color::Yellow,
    }
}
