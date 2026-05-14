//! Renders the conversation history.
//!
//! Each entry in the chat log is displayed with a distinct visual style so the user
//! can tell them apart at a glance:
//!
//! - **User messages** appear as white text on a dark gray background block.
//! - **System messages** appear muted in dark gray.
//! - **Actor messages** appear highlighted with the actor's name and content.
//! - **Assistant messages** appear in white with no background.
//! - **Tool calls** appear as dark text on a dark green background block.
//! - **Tool results** appear as dark text on a dark green (success) or dark red
//!   (failure) background block.
//!
//! A 2-column gutter on the left shows a dark gray background by default,
//! and turns yellow when the cursor selects an entry. Pinned entries show
//! a 📌 emoji in the gutter.
//!
//! The gutter is rendered as a separate column from the content so that
//! line wrapping does not break the gutter display.
//!
//! Text wraps within the available space.

use crate::common::app_state::AppState;
use crate::common::ui_element::UiElement;
use crate::protocol::ChatEntryKind;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use super::shared::{RenderContext, GUTTER_WIDTH};
use super::{actor, assistant, error_entry, system, table, thinking, tool_call, tool_result, user};

/// Default number of lines to show for tool result entries before truncating.
const DEFAULT_TOOL_RESULT_MAX_LINES: u16 = 5;

/// Gutter background color.
const GUTTER_BG: Color = Color::Rgb(0x19, 0x1B, 0x1E);

/// Display element for the full conversation history.
#[derive(Debug)]
pub struct ChatLogElement;

impl UiElement<AppState> for ChatLogElement {
    fn name(&self) -> String {
        "chat-log".to_owned()
    }

    fn is_selectable(&self) -> bool {
        true
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        // Show loading indicator when a session is being loaded.
        if state.session.session_loading {
            let loading = Paragraph::new("Loading session...")
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().fg(Color::DarkGray))
                .block(Block::default().borders(Borders::NONE));
            frame.render_widget(loading, area);
            return;
        }

        let selected_idx = state.active_session().selected_entry_index();
        let history = state.active_session().history();

        // Split area into gutter and content columns.
        let gutter_area = Rect {
            x: area.x,
            y: area.y,
            width: GUTTER_WIDTH,
            height: area.height,
        };
        let content_area = Rect {
            x: area.x + GUTTER_WIDTH,
            y: area.y,
            width: area.width.saturating_sub(GUTTER_WIDTH),
            height: area.height,
        };

        let content_width = content_area.width;

        // Build lines while tracking per-entry wrapped line ranges.
        // entry_line_ranges[i] = (start_wrapped_line, end_wrapped_line) in wrapped coords.
        let mut content_lines: Vec<Line> = Vec::new();
        let mut gutter_lines: Vec<Line> = Vec::new();
        let mut entry_line_ranges: Vec<(u16, u16)> = Vec::with_capacity(history.len());
        let mut wrapped_cursor: u16 = 0;

        for (i, entry) in history.iter().enumerate() {
            let is_selected = selected_idx == Some(i);
            let is_expanded = state.active_session().is_entry_expanded(&entry.id);
            let max_lines = state
                .frontend
                .preferences
                .tool_result_max_lines
                .unwrap_or(DEFAULT_TOOL_RESULT_MAX_LINES);

            let ctx = RenderContext {
                content_width,
                _is_selected: is_selected,
                is_pinned: entry.pin_position.is_some(),
                is_expanded,
                tool_result_max_lines: max_lines,
            };

            let entry_content_lines = entry_to_lines(entry, &ctx);

            // Build gutter lines for this entry (one gutter line per content line).
            let gutter_style = if is_selected {
                Style::default().bg(Color::Yellow)
            } else {
                Style::default().bg(GUTTER_BG)
            };
            let gutter_content = if ctx.is_pinned { "📌" } else { "  " };

            // Count wrapped lines using content_width (wrapping happens in content area).
            let entry_wrapped: u16 = entry_content_lines
                .iter()
                .map(|line| {
                    let w = line.width() as u16;
                    if content_width == 0 || w == 0 {
                        1
                    } else {
                        w.div_ceil(content_width).max(1)
                    }
                })
                .sum();

            let start = wrapped_cursor;
            let end = wrapped_cursor + entry_wrapped;
            entry_line_ranges.push((start, end));
            wrapped_cursor = end;

            // For each content line, emit one gutter line per visual wrapped line.
            // We can't know exact wrap points, but we emit `wrapped_count` gutter
            // lines for this entry so the gutter total matches the content total.
            // Since we can't predict exact wrap points, we emit gutter lines per
            // *logical* content line and rely on the fact that the gutter Paragraph
            // won't wrap (gutter lines are always 2 chars in a 2-wide area).
            //
            // Actually, the simplest correct approach: emit one gutter line per
            // logical content line. When the content Paragraph wraps a line, it
            // creates extra visual rows, but the gutter Paragraph (with 2-char lines
            // in a 2-wide area) won't wrap. The scroll offset keeps them in sync
            // because both use the same scroll value.
            //
            // The key insight: we need gutter_lines.len() == total number of visual
            // (wrapped) content lines. We can compute this by padding gutter_lines
            // to match the wrapped line count.
            let mut entry_gutter_lines = Vec::new();
            for _ in &entry_content_lines {
                entry_gutter_lines
                    .push(Line::from(Span::styled(gutter_content.to_owned(), gutter_style)));
            }

            // If any content line wraps to >1 visual row, the gutter will be short.
            // Pad gutter lines to match the wrapped count.
            let logical_count = entry_content_lines.len() as u16;
            if entry_wrapped > logical_count {
                let extra = entry_wrapped - logical_count;
                for _ in 0..extra {
                    entry_gutter_lines.push(Line::from(Span::styled(
                        "  ".to_owned(),
                        gutter_style,
                    )));
                }
            }

            content_lines.extend(entry_content_lines);
            gutter_lines.extend(entry_gutter_lines);
        }

        let total_wrapped = wrapped_cursor;

        // Bottom-align: when content fits within the viewport, prepend blank lines
        // so messages appear at the bottom with empty space above.
        let blank_count = area.height.saturating_sub(total_wrapped) as usize;

        let mut display_content = Vec::with_capacity(blank_count + content_lines.len());
        let mut display_gutter = Vec::with_capacity(blank_count + gutter_lines.len());
        for _ in 0..blank_count {
            display_content.push(Line::from(""));
            display_gutter.push(Line::from(Span::styled(
                "  ".to_owned(),
                Style::default().bg(GUTTER_BG),
            )));
        }
        display_content.extend(content_lines);
        display_gutter.extend(gutter_lines);

        let scroll_offset = state.active_session().scroll_offset();

        // Clamp scroll_offset: when padded to fill, max_offset is 0 (no scrolling).
        // When content overflows, allow scrolling up to total − viewport height.
        let total_display = total_wrapped + blank_count as u16;
        let max_offset = total_display.saturating_sub(area.height);

        // Feed max_offset back to state so scroll handlers can resolve "at bottom".
        state.active_session().set_last_max_offset(max_offset);

        // Resolve scroll offset: None means "show bottom" → use max_offset.
        let resolved = scroll_offset.unwrap_or(max_offset);
        let mut clamped = resolved.min(max_offset);

        // Scroll-to-selected: adjust clamped offset to keep selected entry visible.
        if let Some(sel_idx) = selected_idx
            && let Some(&(start, end)) = entry_line_ranges.get(sel_idx)
        {
            let abs_start = start + blank_count as u16;
            let abs_end = end + blank_count as u16;
            let viewport_top = clamped;
            let viewport_bottom = clamped.saturating_add(area.height);

            if abs_start < viewport_top {
                clamped = abs_start;
            } else if abs_end > viewport_bottom {
                clamped = abs_end.saturating_sub(area.height);
            } else {
                /* no scroll adjustment needed */
            }
        }

        // Render gutter column.
        let gutter_widget = Paragraph::new(display_gutter)
            .block(Block::default().borders(Borders::NONE))
            .scroll((clamped, 0));
        frame.render_widget(gutter_widget, gutter_area);

        // Render content column.
        let chat_widget = Paragraph::new(display_content)
            .block(Block::default().borders(Borders::NONE))
            .wrap(Wrap { trim: false })
            .scroll((clamped, 0));
        frame.render_widget(chat_widget, content_area);

        // Render a scroll indicator when the user has scrolled up from the bottom.
        if clamped < max_offset {
            let hidden = max_offset - clamped;
            let label = format!(" ↑ {hidden} lines above ");
            let label_len = label.len();
            let indicator = Paragraph::new(Line::from(Span::styled(
                label,
                Style::default().fg(Color::DarkGray).bg(Color::Black),
            )));
            // Render in the bottom-right corner of the chat area.
            let indicator_width = u16::try_from(label_len)
                .unwrap_or(area.width)
                .min(area.width);
            let indicator_area = Rect {
                x: area.x + area.width.saturating_sub(indicator_width),
                y: area.y + area.height.saturating_sub(1),
                width: indicator_width,
                height: 1,
            };
            frame.render_widget(indicator, indicator_area);
        }
    }
}

/// Convert a chat entry into one or more visual lines, splitting on `\n`.
///
/// Each entry type is delegated to its own submodule. Lines returned here are
/// content-width only — the gutter is rendered as a separate column.
fn entry_to_lines(
    entry: &crate::protocol::ChatEntry,
    ctx: &RenderContext,
) -> Vec<Line<'static>> {
    match &entry.kind {
        ChatEntryKind::User(text) => user::to_lines(text, ctx),
        ChatEntryKind::System(text) => system::to_lines(text, ctx),
        ChatEntryKind::Error(text) => error_entry::to_lines(text, ctx),
        ChatEntryKind::Actor { source, text } => actor::to_lines(source, text, ctx),
        ChatEntryKind::Assistant(text) => assistant::to_lines(text, ctx),
        ChatEntryKind::ToolCall {
            name, arguments, ..
        } => tool_call::to_lines(name, arguments, ctx),
        ChatEntryKind::ToolResult {
            name,
            content,
            success,
            ..
        } => tool_result::to_lines(name, content, *success, ctx),
        ChatEntryKind::Table(data) => table::to_lines(data, ctx),
        ChatEntryKind::Thinking(text) => thinking::to_lines(text, ctx),
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::ChatEntry;
    use nullslop_testutil::setup_term;

    use super::*;
    use crate::common::app_state::AppState;
    use crate::protocol::TableData;
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::Span;

    // --- Gutter width constant for test offsets ---
    const G: u16 = GUTTER_WIDTH; // = 2

    #[rstest::rstest]
    fn name_returns_chat_log() {
        // Given a ChatLogElement.
        let element = ChatLogElement;

        // When querying the name.
        let name = element.name();

        // Then it is "chat-log".
        assert_eq!(name, "chat-log");
    }

    #[rstest::rstest]
    fn render_user_entry_has_white_text_on_gray_bg() {
        // Given a ChatLogElement with a user entry "hello".
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::user("hello"));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the content cell (after gutter) has white fg and user bg.
        let buffer = terminal.backend().buffer().clone();
        let content_cell = buffer.cell((G, 9)).expect("cell should exist");
        assert_eq!(content_cell.symbol(), "h");
        assert_eq!(content_cell.style().fg, Some(Color::White));
        assert_eq!(
            content_cell.style().bg,
            Some(Color::Rgb(0x34, 0x35, 0x41))
        );
    }

    #[rstest::rstest]
    fn render_system_entry() {
        // Given a ChatLogElement with a system entry "ready".
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut()
                .push_entry(ChatEntry::system("ready"));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the text is dark gray on the bottom row (in content area).
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((G, 9)).expect("cell should exist");
        assert_eq!(cell.symbol(), "r");
        assert_eq!(cell.style().fg, Some(Color::DarkGray));
    }

    #[rstest::rstest]
    fn render_actor_entry() {
        // Given a ChatLogElement with an actor entry.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut()
                .push_entry(ChatEntry::actor("echo", "HELLO"));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the text starts with "[" (from "[actor]") in the content area and is yellow.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((G, 9)).expect("cell should exist");
        assert_eq!(cell.symbol(), "[");
        assert_eq!(cell.style().fg, Some(Color::Yellow));
    }

    #[rstest::rstest]
    fn render_assistant_entry_is_white() {
        // Given a ChatLogElement with an assistant entry "hello world".
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut()
                .push_entry(ChatEntry::assistant("hello world"));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the content area has the text in white.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((G, 9)).expect("cell should exist");
        assert_eq!(cell.symbol(), "h");
        assert_eq!(cell.style().fg, Some(Color::White));
    }

    #[rstest::rstest]
    fn render_user_continuation_has_block_bg() {
        // Given a ChatLogElement with a user entry containing "hello\nworld".
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut()
                .push_entry(ChatEntry::user("hello\nworld"));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then line 9 has "world" with white fg and user bg (BLOCK continues).
        let buffer = terminal.backend().buffer().clone();
        let w_cell = buffer.cell((G, 9)).expect("cell should exist");
        assert_eq!(w_cell.symbol(), "w");
        assert_eq!(w_cell.style().fg, Some(Color::White));
        assert_eq!(w_cell.style().bg, Some(Color::Rgb(0x34, 0x35, 0x41)));
    }

    #[rstest::rstest]
    fn render_assistant_continuation_has_no_bg() {
        // Given a ChatLogElement with an assistant entry containing "line1\nline2".
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut()
                .push_entry(ChatEntry::assistant("line1\nline2"));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // And line 9 has "line2" (no prefix, white, no bg).
        let buffer = terminal.backend().buffer().clone();
        let l_cell = buffer.cell((G, 9)).expect("cell should exist");
        assert_eq!(l_cell.symbol(), "l");
        assert_eq!(l_cell.style().fg, Some(Color::White));
    }

    #[rstest::rstest]
    fn render_few_messages_bottom_aligned() {
        // Given a ChatLogElement with one user entry in a 40x10 viewport.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::user("hello"));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the bottom row content has user text.
        let buffer = terminal.backend().buffer().clone();
        let bottom_cell = buffer.cell((G, 9)).expect("cell should exist");
        assert_eq!(bottom_cell.symbol(), "h");
    }

    #[rstest::rstest]
    fn chat_log_element_is_selectable() {
        // Given a ChatLogElement.
        let element = ChatLogElement;

        // When calling is_selectable.
        let selectable: &dyn UiElement<AppState> = &element;

        // Then it returns true.
        assert!(selectable.is_selectable());
    }

    #[rstest::rstest]
    fn selected_entry_gutter_is_yellow() {
        // Given a ChatLogElement with 2 entries, first selected.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::user("hello"));
            s.active_session_mut().push_entry(ChatEntry::user("world"));
            s.active_session_mut().select_next_entry(); // selects index 0
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the selected entry's gutter has yellow bg.
        let buffer = terminal.backend().buffer().clone();
        let gutter_cell = buffer.cell((0, 8)).expect("cell should exist");
        assert_eq!(gutter_cell.style().bg, Some(Color::Yellow));

        // And the unselected entry's gutter has dark gray bg.
        let unselected_gutter = buffer.cell((0, 9)).expect("cell should exist");
        assert_eq!(unselected_gutter.style().bg, Some(GUTTER_BG));
    }

    #[rstest::rstest]
    fn render_no_selection_has_dark_gray_gutter() {
        // Given a ChatLogElement with entries but no selection.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::user("hello"));
            s.active_session_mut().push_entry(ChatEntry::user("world"));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the gutter has dark gray bg (not yellow).
        let buffer = terminal.backend().buffer().clone();
        let gutter_cell = buffer.cell((0, 8)).expect("cell should exist");
        assert_eq!(gutter_cell.style().bg, Some(GUTTER_BG));
    }

    #[rstest::rstest]
    fn render_pinned_entry_shows_pin_in_gutter() {
        // Given a ChatLogElement with one pinned user entry.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut()
                .push_entry(ChatEntry::user("hello").with_pin(crate::protocol::PinPosition::Top));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the gutter contains the 📌 character.
        let buffer = terminal.backend().buffer().clone();
        let has_pin = (0..10).any(|row| {
            (0..2).any(|col| {
                buffer
                    .cell((col, row))
                    .is_some_and(|c| c.symbol() == "\u{1F4CC}")
            })
        });
        assert!(
            has_pin,
            "pinned entry should show \u{1F4CC} pin icon in gutter"
        );
    }

    #[rstest::rstest]
    fn render_unpinned_entry_has_no_pin_icon() {
        // Given a ChatLogElement with one unpinned user entry.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::user("hello"));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then no cell in the buffer contains the 📌 character.
        let buffer = terminal.backend().buffer().clone();
        let has_pin = (0..10).any(|row| {
            (0..40).any(|col| {
                buffer
                    .cell((col, row))
                    .is_some_and(|c| c.symbol() == "\u{1F4CC}")
            })
        });
        assert!(
            !has_pin,
            "unpinned entry should not show \u{1F4CC} pin icon"
        );
    }

    #[rstest::rstest]
    fn render_scroll_to_selected_keeps_entry_visible() {
        // Given a ChatLogElement with many entries where the first is selected
        // and the viewport is small enough that it would normally be scrolled off.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            // Add 20 entries (each 1 line).
            for i in 0..20 {
                s.active_session_mut()
                    .push_entry(ChatEntry::user(format!("msg {i}")));
            }
            // Select the first entry (index 0).
            s.active_session_mut().select_next_entry(); // selects index 0
            s
        };

        let (mut terminal, area) = setup_term(40, 5); // 5-line viewport

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the selected entry's gutter (yellow) should be visible in the viewport.
        let buffer = terminal.backend().buffer().clone();
        let has_yellow_gutter = (0..5).any(|row| {
            buffer
                .cell((0, row))
                .is_some_and(|c| c.style().bg == Some(Color::Yellow))
        });
        assert!(
            has_yellow_gutter,
            "selected entry should be visible in viewport when scroll-to-selected is active"
        );
    }

    #[rstest::rstest]
    fn render_table_entry_has_bold_headers() {
        // Given a ChatLogElement with a table entry.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            let data = TableData {
                headers: vec![
                    Span::raw("Provider"),
                    Span::raw("Count"),
                    Span::raw("Status"),
                ],
                rows: vec![vec![
                    Span::raw("ollama"),
                    Span::raw("5"),
                    Span::styled("\u{2705}", Style::default().fg(Color::Green)),
                ]],
            };
            s.active_session_mut().push_entry(ChatEntry::table(data));
            s
        };

        let (mut terminal, area) = setup_term(60, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the header row contains "P" (from "Provider") with bold styling.
        let buffer = terminal.backend().buffer().clone();
        let has_bold_header = (0..10).any(|row| {
            (G..60).any(|col| {
                buffer.cell((col, row)).is_some_and(|c| {
                    c.symbol() == "P" && c.style().add_modifier.contains(Modifier::BOLD)
                })
            })
        });
        assert!(has_bold_header, "table header should be bold");
    }

    #[rstest::rstest]
    fn render_table_entry_has_data_rows() {
        // Given a ChatLogElement with a table entry containing data rows.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            let data = TableData {
                headers: vec![
                    Span::raw("Provider"),
                    Span::raw("Count"),
                    Span::raw("Status"),
                ],
                rows: vec![vec![
                    Span::raw("ollama"),
                    Span::raw("5"),
                    Span::styled("\u{2705}", Style::default().fg(Color::Green)),
                ]],
            };
            s.active_session_mut().push_entry(ChatEntry::table(data));
            s
        };

        let (mut terminal, area) = setup_term(60, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the buffer contains "ollama" somewhere (a data row cell).
        let buffer = terminal.backend().buffer().clone();
        let has_ollama = (0..10).any(|row| {
            (G..60).any(|col| buffer.cell((col, row)).is_some_and(|c| c.symbol() == "o"))
        });
        assert!(has_ollama, "table data row should contain 'ollama'");
    }

    #[rstest::rstest]
    fn render_table_entry_has_separator_line() {
        // Given a ChatLogElement with a table entry.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            let data = TableData {
                headers: vec![Span::raw("Provider"), Span::raw("Count")],
                rows: vec![vec![Span::raw("test"), Span::raw("1")]],
            };
            s.active_session_mut().push_entry(ChatEntry::table(data));
            s
        };

        let (mut terminal, area) = setup_term(60, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the buffer contains a separator line with \u{2500} (─).
        let buffer = terminal.backend().buffer().clone();
        let has_separator = (0..10).any(|row| {
            (G..60).any(|col| {
                buffer
                    .cell((col, row))
                    .is_some_and(|c| c.symbol() == "\u{2500}")
            })
        });
        assert!(has_separator, "table should have a separator line");
    }

    #[rstest::rstest]
    fn render_error_entry() {
        // Given a ChatLogElement with an error entry "Cancelled".
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut()
                .push_entry(ChatEntry::error("Cancelled"));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the text is red in the content area.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((G, 9)).expect("cell should exist");
        assert_eq!(cell.symbol(), "C");
        assert_eq!(cell.style().fg, Some(Color::Red));
    }

    #[rstest::rstest]
    fn render_thinking_entry_is_dark_gray() {
        // Given a ChatLogElement with a thinking entry "reasoning here".
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut()
                .push_entry(ChatEntry::thinking("reasoning here"));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the text is dark gray in the content area.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((G, 9)).expect("cell should exist");
        assert_eq!(cell.symbol(), "r");
        assert_eq!(cell.style().fg, Some(Color::DarkGray));
    }

    #[rstest::rstest]
    fn render_thinking_entry_appears_above_assistant() {
        // Given a ChatLogElement with thinking then assistant entries.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut()
                .push_entry(ChatEntry::thinking("reasoning"));
            s.active_session_mut()
                .push_entry(ChatEntry::assistant("response"));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the thinking entry is dark gray and appears above the assistant entry.
        let buffer = terminal.backend().buffer().clone();
        // Line 8 has the thinking entry (dark gray).
        let thinking_cell = buffer.cell((G, 8)).expect("cell should exist");
        assert_eq!(thinking_cell.symbol(), "r");
        assert_eq!(thinking_cell.style().fg, Some(Color::DarkGray));
        // Line 9 has the assistant entry (white).
        let assistant_cell = buffer.cell((G, 9)).expect("cell should exist");
        assert_eq!(assistant_cell.symbol(), "r");
        assert_eq!(assistant_cell.style().fg, Some(Color::White));
    }

    #[rstest::rstest]
    fn render_tool_result_truncated_shows_new_format() {
        // Given a ChatLogElement with a tool result that has 10 lines.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            let content = (1..=10)
                .map(|i| format!("line {i}"))
                .collect::<Vec<_>>()
                .join("\n");
            s.active_session_mut().push_entry(ChatEntry::tool_result(
                "id", "bash", content, true,
            ));
            s
        };

        let (mut terminal, area) = setup_term(80, 20);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the buffer contains "---(5 more lines)---" somewhere.
        let buffer = terminal.backend().buffer().clone();
        let has_indicator = (0..20).any(|row| {
            let row_text: String = (G..80)
                .map(|col| buffer.cell((col, row)).map(|c| c.symbol()).unwrap_or(" "))
                .collect();
            row_text.contains("---(5 more lines)---")
        });
        assert!(
            has_indicator,
            "truncated tool result should show '---(5 more lines)---'"
        );
    }

    #[rstest::rstest]
    fn render_tool_result_expanded_shows_full_content() {
        // Given a ChatLogElement with an expanded tool result that has 10 lines.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            let content = (1..=10)
                .map(|i| format!("line {i}"))
                .collect::<Vec<_>>()
                .join("\n");
            s.active_session_mut().push_entry(ChatEntry::tool_result(
                "id", "bash", content, true,
            ));
            // Expand the entry.
            let entry_id = s.active_session().history()[0].id.clone();
            s.active_session_mut().toggle_expand_entry(entry_id);
            s
        };

        let (mut terminal, area) = setup_term(80, 20);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the buffer contains "line 10" (the last line, not truncated).
        let buffer = terminal.backend().buffer().clone();
        let has_last_line = (0..20).any(|row| {
            let row_text: String = (G..80)
                .map(|col| buffer.cell((col, row)).map(|c| c.symbol()).unwrap_or(" "))
                .collect();
            row_text.contains("line 10")
        });
        assert!(has_last_line, "expanded tool result should show 'line 10'");

        // And no truncation indicator.
        let has_indicator = (0..20).any(|row| {
            let row_text: String = (G..80)
                .map(|col| buffer.cell((col, row)).map(|c| c.symbol()).unwrap_or(" "))
                .collect();
            row_text.contains("more lines")
        });
        assert!(
            !has_indicator,
            "expanded tool result should not show truncation indicator"
        );
    }

    #[rstest::rstest]
    fn render_tool_result_short_not_truncated() {
        // Given a ChatLogElement with a tool result that has only 3 lines.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            let content = "line 1\nline 2\nline 3".to_owned();
            s.active_session_mut().push_entry(ChatEntry::tool_result(
                "id", "bash", content, true,
            ));
            s
        };

        let (mut terminal, area) = setup_term(80, 20);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then no truncation indicator appears.
        let buffer = terminal.backend().buffer().clone();
        let has_indicator = (0..20).any(|row| {
            let row_text: String = (G..80)
                .map(|col| buffer.cell((col, row)).map(|c| c.symbol()).unwrap_or(" "))
                .collect();
            row_text.contains("more lines")
        });
        assert!(
            !has_indicator,
            "short tool result should not be truncated"
        );
    }

    #[rstest::rstest]
    fn render_tool_call_has_dark_green_bg() {
        // Given a ChatLogElement with a tool call entry.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::tool_call(
                "id", "bash", "ls -la",
            ));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the content area has the updated tool call bg.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((G, 9)).expect("cell should exist");
        assert_eq!(cell.style().bg, Some(Color::Rgb(0x28, 0x32, 0x28)));
    }

    #[rstest::rstest]
    fn render_tool_result_success_has_dark_green_bg() {
        // Given a ChatLogElement with a successful tool result entry.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::tool_result(
                "id", "bash", "output", true,
            ));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the content area has the updated tool result success bg.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((G, 9)).expect("cell should exist");
        assert_eq!(cell.style().bg, Some(Color::Rgb(0x28, 0x32, 0x28)));
    }

    #[rstest::rstest]
    fn render_tool_result_failure_has_dark_red_bg() {
        // Given a ChatLogElement with a failed tool result entry.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::tool_result(
                "id", "bash", "error", false,
            ));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the content area has the updated tool result failure bg.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((G, 9)).expect("cell should exist");
        assert_eq!(cell.style().bg, Some(Color::Rgb(0x3C, 0x28, 0x28)));
    }

    #[rstest::rstest]
    fn render_user_block_bg_fills_full_width() {
        // Given a ChatLogElement with a short user entry.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::user("hi"));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then cells at the right edge of the content area have user bg (BLOCK fill).
        let buffer = terminal.backend().buffer().clone();
        let right_cell = buffer.cell((39, 9)).expect("cell should exist");
        assert_eq!(right_cell.style().bg, Some(Color::Rgb(0x34, 0x35, 0x41)));
    }

    #[rstest::rstest]
    fn render_tool_result_name_on_first_line() {
        // Given a ChatLogElement with a tool result.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::tool_result(
                "id", "bash", "output", true,
            ));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // With 1 entry of 2 lines (name + content) in a 10-row viewport,
        // name is at row 8, content at row 9.
        let buffer = terminal.backend().buffer().clone();
        let name_cell = buffer.cell((G, 8)).expect("cell should exist");
        assert_eq!(name_cell.symbol(), "b");
        assert_eq!(name_cell.style().fg, Some(Color::Rgb(0x3A, 0x3A, 0x3A)));
        let content_cell = buffer.cell((G, 9)).expect("cell should exist");
        assert_eq!(content_cell.symbol(), "o");
    }

    #[rstest::rstest]
    fn render_truncation_indicator_is_correct_fg() {
        // Given a ChatLogElement with a tool result that gets truncated.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            let content = (1..=10)
                .map(|i| format!("line {i}"))
                .collect::<Vec<_>>()
                .join("\n");
            s.active_session_mut().push_entry(ChatEntry::tool_result(
                "id", "bash", content, true,
            ));
            s
        };

        let (mut terminal, area) = setup_term(80, 20);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the truncation indicator line has the correct fg.
        let buffer = terminal.backend().buffer().clone();
        let truncation_row = (0..20).find(|&row| {
            (G..80).any(|col| {
                buffer
                    .cell((col, row))
                    .is_some_and(|buf_cell| {
                        buf_cell.symbol() == "-"
                            && buf_cell.style().fg == Some(Color::Rgb(0x53, 0x53, 0x53))
                    })
            })
        });
        assert!(
            truncation_row.is_some(),
            "truncation indicator should have correct fg"
        );
    }

    #[rstest::rstest]
    fn render_gutter_is_dark_gray_by_default() {
        // Given a ChatLogElement with entries.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::user("hello"));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the gutter cells have dark gray bg.
        let buffer = terminal.backend().buffer().clone();
        let gutter_cell = buffer.cell((0, 9)).expect("cell should exist");
        assert_eq!(gutter_cell.style().bg, Some(GUTTER_BG));
    }

    #[rstest::rstest]
    fn gutter_persists_across_wrapped_lines() {
        // Given a ChatLogElement with a long assistant entry that wraps.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            // Create a long line that will wrap in a 20-wide content area.
            let long_text = "abcdefghijklmnopqrstuvwxyz";
            s.active_session_mut()
                .push_entry(ChatEntry::assistant(long_text));
            s
        };

        let (mut terminal, area) = setup_term(22, 10); // 22 = 2 gutter + 20 content

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the gutter column has dark gray bg on both the original line
        // and the wrapped continuation line.
        let buffer = terminal.backend().buffer().clone();

        // The entry should be at rows 8-9 (bottom-aligned, 2 visual lines).
        // Both rows should have gutter cells with dark gray bg.
        for row in [8, 9] {
            let gutter_cell = buffer.cell((0, row)).expect("cell should exist");
            assert_eq!(
                gutter_cell.style().bg,
                Some(GUTTER_BG),
                "gutter at row {row} should have gutter bg"
            );
        }
    }

    #[rstest::rstest]
    fn tool_call_fg_is_correct() {
        // Given a ChatLogElement with a tool call entry.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::tool_call(
                "id", "bash", "ls",
            ));
            s
        };

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the tool call text has the updated fg color.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((G, 9)).expect("cell should exist");
        assert_eq!(cell.style().fg, Some(Color::Rgb(0x3A, 0x3A, 0x3A)));
    }
}
