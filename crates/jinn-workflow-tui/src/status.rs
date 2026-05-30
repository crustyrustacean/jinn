//! Status indicator symbols and colors.
//!
//! Maps [`NodeStatus`](jinn_workflow::engine::NodeStatus) to visual
//! indicators: empty circle for pending, filled colored circles for
//! terminal states, and braille spinner frames for running.

use jinn_workflow::engine::NodeStatus;
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
        NodeStatus::AwaitingInput => "✎",
    }
}

/// Returns the color for a node status indicator.
///
/// - `Pending` → [`Color::DarkGray`]
/// - `Running` → [`Color::Cyan`]
/// - `Completed` → [`Color::Green`]
/// - `Failed` → [`Color::Red`]
/// - `Skipped` → [`Color::Yellow`]
/// - `AwaitingInput` → `awaiting_input_color`
#[must_use]
pub fn status_color(status: NodeStatus, awaiting_input_color: Color) -> Color {
    match status {
        NodeStatus::Pending => Color::DarkGray,
        NodeStatus::Running => Color::Cyan,
        NodeStatus::AwaitingInput => awaiting_input_color,
        NodeStatus::Completed => Color::Green,
        NodeStatus::Failed => Color::Red,
        NodeStatus::Skipped => Color::Yellow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Kills: status_symbol % -> /
    #[test]
    fn status_symbol_running_uses_modulo_for_spinner_index() {
        // tick=255, SPINNER_FRAMES has 10 entries.
        // With %: index = 255 % 10 = 5 (frame "⠴")
        // With /: index = 255 / 10 = 25 (panic or OOB)
        let sym = status_symbol(NodeStatus::Running, 255);
        assert_eq!(sym, "⠴", "tick=255 must select frame 5 via modulo, not frame 25 via division");
    }

    // Kills: status_symbol returning wrong string for each status
    #[test]
    fn status_symbol_pending_returns_circle() {
        assert_eq!(status_symbol(NodeStatus::Pending, 0), "○");
    }

    #[test]
    fn status_symbol_completed_returns_filled_circle() {
        assert_eq!(status_symbol(NodeStatus::Completed, 0), "●");
    }

    #[test]
    fn status_symbol_failed_returns_filled_circle() {
        assert_eq!(status_symbol(NodeStatus::Failed, 0), "●");
    }

    #[test]
    fn status_symbol_skipped_returns_filled_circle() {
        assert_eq!(status_symbol(NodeStatus::Skipped, 0), "●");
    }

    #[test]
    fn status_symbol_awaiting_input_returns_pencil() {
        assert_eq!(status_symbol(NodeStatus::AwaitingInput, 0), "✎");
    }

    // Kills: status_color -> Default (always returning default color)
    #[test]
    fn status_color_pending_returns_dark_gray() {
        assert_eq!(status_color(NodeStatus::Pending, Color::Magenta), Color::DarkGray);
    }

    #[test]
    fn status_color_running_returns_cyan() {
        assert_eq!(status_color(NodeStatus::Running, Color::Magenta), Color::Cyan);
    }

    #[test]
    fn status_color_completed_returns_green() {
        assert_eq!(status_color(NodeStatus::Completed, Color::Magenta), Color::Green);
    }

    #[test]
    fn status_color_failed_returns_red() {
        assert_eq!(status_color(NodeStatus::Failed, Color::Magenta), Color::Red);
    }

    #[test]
    fn status_color_skipped_returns_yellow() {
        assert_eq!(status_color(NodeStatus::Skipped, Color::Magenta), Color::Yellow);
    }

    #[test]
    fn status_color_awaiting_input_returns_passed_color() {
        assert_eq!(status_color(NodeStatus::AwaitingInput, Color::Magenta), Color::Magenta);
    }
}
