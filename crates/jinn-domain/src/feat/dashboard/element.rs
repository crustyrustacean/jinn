//! Renders the dashboard view - a list of actors with their startup status.
//!
//! Each actor is displayed with a 2-cell left border. The selected entry shows
//! a solid yellow full block (`██`) in the border; unselected entries show spaces.
//! The view scrolls when actors overflow the viewport, keeping the selected
//! entry visible.

use crate::common::app_state::AppState;
use crate::common::render_ctx::RenderCtx;
use crate::common::ui_element::UiElement;
use crate::feat::dashboard::ActorStatus;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// Solid yellow full block used as the selection indicator.
const SELECTED_INDICATOR: &str = "\u{2588}\u{2588}";
/// Two spaces used as the unselected border.
const UNSELECTED_BORDER: &str = "  ";

/// Display element for the actor dashboard.
#[derive(Debug)]
pub struct DashboardElement;

impl UiElement for DashboardElement {
    fn name(&self) -> String {
        "dashboard".to_owned()
    }

    fn is_selectable(&self) -> bool {
        true
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
        let state = ctx.state;
        let theme = &state.frontend.theme;
        let actors = state.frontend.dashboard.actors();
        let selected_index = state.frontend.dashboard.selected_index();

        let lines: Vec<Line> = if actors.is_empty() {
            vec![Line::from(Span::styled(
                "No actors registered.",
                Style::default().fg(theme.muted_text),
            ))]
        } else {
            let mut lines = Vec::new();

            for (i, entry) in actors.iter().enumerate() {
                let is_selected = i == selected_index;
                let border_span = if is_selected {
                    Span::styled(SELECTED_INDICATOR, Style::default().fg(theme.focus_accent))
                } else {
                    Span::raw(UNSELECTED_BORDER)
                };

                let (label, color) = match entry.status {
                    ActorStatus::Starting => ("Starting", theme.warning),
                    ActorStatus::Running => ("Running", theme.success),
                };

                // Name line: border + padded name ... status
                lines.push(Line::from(vec![
                    border_span,
                    Span::styled(format!(" {}", entry.name), Style::default().bold()),
                    // Fill with spaces to push status right
                    Span::raw(fill_to_status(&entry.name, area.width)),
                    Span::styled(label, Style::default().fg(color)),
                ]));

                // Description line (if present).
                if let Some(desc) = &entry.description {
                    let desc_border = if is_selected {
                        Span::styled(SELECTED_INDICATOR, Style::default().fg(theme.focus_accent))
                    } else {
                        Span::raw(UNSELECTED_BORDER)
                    };
                    lines.push(Line::from(vec![
                        desc_border,
                        Span::styled(format!("   {desc}"), Style::default().fg(theme.muted_text)),
                    ]));
                }

                // Blank line between actors (not after the last one).
                if i < actors.len() - 1 {
                    lines.push(Line::from(""));
                }
            }

            lines
        };

        // Calculate total visual lines for scroll clamping.
        let total_lines = lines.len() as u16;
        let max_offset = total_lines.saturating_sub(area.height);
        let scroll_offset = state.frontend.dashboard.scroll_offset().min(max_offset);

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::NONE))
            .scroll((scroll_offset, 0));
        frame.render_widget(widget, area);
    }
}

/// Returns spaces to pad between the name and the right-aligned status.
/// The status label takes up to ~8 chars ("Starting"), so we leave room.
/// The 2-cell left border is accounted for in the calculation.
fn fill_to_status(name: &str, area_width: u16) -> String {
    let status_width: usize = 8; // "Starting" is the longest status
    let border_width: usize = 2; // "██" or "  "
    let name_len = name.len() + 1 + border_width; // +1 for leading space, +2 for border
    let available = area_width as usize;
    let padding = available
        .saturating_sub(name_len)
        .saturating_sub(status_width);
    " ".repeat(padding.max(1))
}
