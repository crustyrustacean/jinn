//! Renders the pinned context panel — lists pinned entries with position badges.
//!
//! Displays all pinned entries from the active session. Each entry shows a pin
//! position badge ([TOP], [BOT], [REL]), an entry type icon, and truncated content.
//! The selected entry is highlighted with a yellow marker and reversed style.
//! When no entries are pinned, shows a dimmed "No pinned entries." message.

use nullslop_component_ui::UiElement;
use nullslop_protocol::{ChatEntryKind, PinPosition};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::AppState;

/// Solid yellow full block used as the selection indicator.
const SELECTED_INDICATOR: &str = "\u{2588}\u{2588}";
/// Two spaces used as the unselected border.
const UNSELECTED_BORDER: &str = "  ";

/// UI element that renders the pinned context panel.
#[derive(Debug)]
pub struct PinnedPanelElement;

impl UiElement<AppState> for PinnedPanelElement {
    fn name(&self) -> String {
        "pinned-panel".to_owned()
    }

    fn is_selectable(&self) -> bool {
        true
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        let pinned = state.active_session().pinned_entries();
        if pinned.is_empty() {
            render_no_entries(frame, area);
            return;
        }

        let selected_index = state.pinned_panel.selection_index();
        let lines = build_entry_list(&pinned, selected_index, area.width);

        let total_lines = lines.len() as u16;
        let max_offset = total_lines.saturating_sub(area.height);
        let scroll_offset = max_offset;

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::NONE))
            .scroll((scroll_offset, 0));
        frame.render_widget(widget, area);
    }
}

/// Renders the "No pinned entries." placeholder.
fn render_no_entries(frame: &mut Frame<'_>, area: Rect) {
    let msg = Paragraph::new("No pinned entries.")
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::NONE));
    frame.render_widget(msg, area);
}

/// Returns the badge text and color for a pin position.
fn position_badge(position: PinPosition) -> (&'static str, Color) {
    match position {
        PinPosition::Top => ("[TOP]", Color::Cyan),
        PinPosition::Bottom => ("[BOT]", Color::Magenta),
        PinPosition::Relative => ("[REL]", Color::DarkGray),
    }
}

/// Returns the display prefix and truncated content for a chat entry kind.
fn entry_prefix_and_content(kind: &ChatEntryKind) -> (&'static str, String) {
    match kind {
        ChatEntryKind::User(text) => ("> ", truncate_str(text, 40)),
        ChatEntryKind::Assistant(text) => ("\u{2666} ", truncate_str(text, 40)),
        ChatEntryKind::System(text) => ("\u{2699} ", truncate_str(text, 40)),
        ChatEntryKind::Actor { source, text } => {
            let content = format!("[{}] {}", source, truncate_str(text, 30));
            ("", content)
        }
        ChatEntryKind::ToolCall { name, .. } => {
            ("\u{2692} ", format!("{}(...)", truncate_str(name, 20)))
        }
        ChatEntryKind::ToolResult {
            name,
            content,
            success,
            ..
        } => {
            let icon = if *success { "\u{2705}" } else { "\u{274c}" };
            (
                "",
                format!(
                    "{} {}: {}",
                    icon,
                    truncate_str(name, 15),
                    truncate_str(content, 20)
                ),
            )
        }
    }
}

/// Truncates a string to the given max length, appending an ellipsis if needed.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_owned()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(1)).collect();
        format!("{truncated}\u{2026}")
    }
}

/// Builds the list of lines for the pinned entries panel.
fn build_entry_list(
    pinned: &[&nullslop_protocol::ChatEntry],
    selected_index: usize,
    _area_width: u16,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // Header
    lines.push(Line::from(vec![Span::styled(
        format!(" Pinned Context \u{2014} {}", pinned.len()),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    for (i, entry) in pinned.iter().enumerate() {
        let is_selected = i == selected_index;

        let border = if is_selected {
            Span::styled(SELECTED_INDICATOR, Style::default().fg(Color::Yellow))
        } else {
            Span::raw(UNSELECTED_BORDER)
        };

        let (badge_text, badge_color) =
            position_badge(entry.pin_position.unwrap_or(PinPosition::Relative));

        let (prefix, content) = entry_prefix_and_content(&entry.kind);

        let style = if is_selected {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };

        lines.push(Line::from(vec![
            border,
            Span::styled(format!(" {badge_text} "), Style::default().fg(badge_color)),
            Span::styled(format!("{prefix}{content}"), style),
        ]));

        if i < pinned.len() - 1 {
            lines.push(Line::from(""));
        }
    }

    lines
}

#[cfg(test)]
mod tests {
    use nullslop_protocol::{ChatEntry, PinPosition};
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::Terminal;

    use super::*;
    use crate::AppState;

    fn state_with_pinned(count: usize) -> AppState {
        let mut state = AppState::default();
        for i in 0..count {
            let entry = ChatEntry::user(format!("pinned message {i}"));
            let entry_id = entry.id.clone();
            state.active_session_mut().push_entry(entry);
            state
                .active_session_mut()
                .pin_entry(&entry_id, PinPosition::Top);
        }
        state
    }

    fn render_rows(
        element: &mut PinnedPanelElement,
        state: &AppState,
        width: u16,
        height: u16,
    ) -> Vec<String> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, width, height);
        terminal
            .draw(|frame| {
                element.render(frame, area, state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| {
                        buffer
                            .cell((x, y))
                            .map_or(" ", ratatui::buffer::Cell::symbol)
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn name_returns_pinned_panel() {
        let element = PinnedPanelElement;
        assert_eq!(element.name(), "pinned-panel");
    }

    #[test]
    fn render_no_entries_shows_message() {
        let mut element = PinnedPanelElement;
        let state = AppState::default();
        let rows = render_rows(&mut element, &state, 40, 10);
        assert!(rows[0].contains("No pinned entries."));
    }

    #[test]
    fn render_shows_pinned_entries() {
        let mut element = PinnedPanelElement;
        let state = state_with_pinned(2);
        let rows = render_rows(&mut element, &state, 60, 20);
        let combined = rows.join("\n");
        assert!(
            combined.contains("pinned message 0"),
            "should contain first entry, got: {combined}"
        );
        assert!(
            combined.contains("pinned message 1"),
            "should contain second entry, got: {combined}"
        );
    }

    #[test]
    fn render_selected_entry_has_yellow_marker() {
        let mut element = PinnedPanelElement;
        let state = state_with_pinned(2);

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 60, 20);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        // First entry at index 0 is selected by default.
        // Header is row 0, blank row 1, entry at row 2.
        let cell0 = buffer.cell((0, 2)).expect("cell 0,2");
        assert_eq!(cell0.symbol(), "\u{2588}");
        assert_eq!(cell0.fg, Color::Yellow);
    }

    #[test]
    fn pinned_panel_element_is_selectable() {
        let element = PinnedPanelElement;
        let selectable: &dyn UiElement<AppState> = &element;
        assert!(selectable.is_selectable());
    }
}
