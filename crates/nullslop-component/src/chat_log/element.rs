//! Renders the conversation history.
//!
//! Each entry in the chat log is displayed with a distinct visual style so the user
//! can tell them apart at a glance:
//!
//! - **User messages** appear bold with a `>` prefix.
//! - **System messages** appear muted with indentation.
//! - **Actor messages** appear highlighted with the actor's name and content.
//!
//! Text wraps within the available space.

use nullslop_component_ui::UiElement;
use nullslop_protocol::ChatEntryKind;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::AppState;

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
        if state.session_loading {
            let loading = Paragraph::new("Loading session...")
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().fg(Color::DarkGray))
                .block(Block::default().borders(Borders::NONE));
            frame.render_widget(loading, area);
            return;
        }

        let selected_idx = state.active_session().selected_entry_index();
        let history = state.active_session().history();

        // Build lines while tracking per-entry wrapped line ranges.
        // entry_line_ranges[i] = (start_wrapped_line, end_wrapped_line) in wrapped coords.
        let mut lines: Vec<Line> = Vec::new();
        let mut entry_line_ranges: Vec<(u16, u16)> = Vec::with_capacity(history.len());
        let mut wrapped_cursor: u16 = 0;

        for (i, entry) in history.iter().enumerate() {
            let is_selected = selected_idx == Some(i);
            let entry_lines = entry_to_lines(entry, is_selected);
            let entry_wrapped: u16 = entry_lines
                .iter()
                .map(|line| {
                    let w = line.width() as u16;
                    if area.width == 0 || w == 0 {
                        1
                    } else {
                        w.div_ceil(area.width).max(1)
                    }
                })
                .sum();
            let start = wrapped_cursor;
            let end = wrapped_cursor + entry_wrapped;
            entry_line_ranges.push((start, end));
            wrapped_cursor = end;
            lines.extend(entry_lines);
        }

        let total_wrapped = wrapped_cursor;

        // Bottom-align: when content fits within the viewport, prepend blank lines
        // so messages appear at the bottom with empty space above.
        let blank_count = area.height.saturating_sub(total_wrapped) as usize;
        let mut display_lines = Vec::with_capacity(blank_count + lines.len());
        for _ in 0..blank_count {
            display_lines.push(Line::from(""));
        }
        display_lines.extend(lines);

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

        let chat_widget = Paragraph::new(display_lines)
            .block(Block::default().borders(Borders::NONE))
            .wrap(Wrap { trim: true })
            .scroll((clamped, 0));
        frame.render_widget(chat_widget, area);

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
/// The first line gets the entry-type prefix; continuation lines get indentation.
/// When `is_selected` is true, the first line gets a `▶` prefix and `REVERSED` style.
fn entry_to_lines(entry: &nullslop_protocol::ChatEntry, is_selected: bool) -> Vec<Line<'static>> {
    let pinned = entry.pin_position.is_some();

    match &entry.kind {
        ChatEntryKind::User(text) => {
            let prefix = if pinned { "📌 > " } else { "> " };
            multiline_styled(
                text,
                prefix,
                "  ",
                Style::default().add_modifier(Modifier::BOLD),
                is_selected,
            )
        }
        ChatEntryKind::System(text) => {
            let prefix = if pinned { "📌   " } else { "  " };
            multiline_styled(
                text,
                prefix,
                "  ",
                Style::default().fg(Color::DarkGray),
                is_selected,
            )
        }
        ChatEntryKind::Actor { source, text } => {
            let base = format!("[actor] {source}: ");
            let prefix = if pinned { format!("📌 {base}") } else { base };
            multiline_styled(
                text,
                &prefix,
                "  ",
                Style::default().fg(Color::Yellow),
                is_selected,
            )
        }
        ChatEntryKind::Assistant(text) => {
            let prefix = if pinned { "📌 ✦ " } else { "✦ " };
            multiline_styled(
                text,
                prefix,
                "  ",
                Style::default().fg(Color::Cyan),
                is_selected,
            )
        }
        ChatEntryKind::ToolCall {
            name, arguments, ..
        } => {
            let prefix = if pinned { "📌 " } else { "  " };
            multiline_styled(
                format!("🔧 {name}({arguments})"),
                prefix,
                "  ",
                Style::default().fg(Color::Magenta),
                is_selected,
            )
        }
        ChatEntryKind::ToolResult {
            name,
            content,
            success,
            ..
        } => {
            let icon = if *success { "✅" } else { "❌" };
            let prefix = if pinned { "📌 " } else { "  " };
            multiline_styled(
                format!("{icon} {name}: {content}"),
                prefix,
                "  ",
                if *success {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::Red)
                },
                is_selected,
            )
        }
    }
}

/// Split text on `\n` and produce styled lines with the given prefix/indent.
///
/// When `is_selected` is true, the first line gets a `▶ ` prefix and
/// `Modifier::REVERSED` added to its style.
fn multiline_styled<T, P, I>(
    text: T,
    prefix: P,
    indent: I,
    style: Style,
    is_selected: bool,
) -> Vec<Line<'static>>
where
    T: AsRef<str>,
    P: AsRef<str>,
    I: AsRef<str>,
{
    let text = text.as_ref();
    let prefix = prefix.as_ref();
    let _ = indent.as_ref();
    let segments = text.split('\n');
    let mut lines = Vec::new();
    for (i, segment) in segments.enumerate() {
        let (content, line_style) = if i == 0 && is_selected {
            (
                format!("▶ {prefix}{segment}"),
                style.add_modifier(Modifier::REVERSED),
            )
        } else if i == 0 {
            (format!("{prefix}{segment}"), style)
        } else {
            (segment.to_owned(), style)
        };
        lines.push(Line::from(Span::styled(content, line_style)));
    }
    lines
}

#[cfg(test)]
mod tests {
    use nullslop_protocol::ChatEntry;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;

    use super::*;
    use crate::AppState;

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
    fn render_empty_history() {
        // Given a ChatLogElement with empty chat history.
        let mut element = ChatLogElement;
        let state = AppState::default();

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then it renders without panic and the first cell is empty.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((0, 0)).expect("cell should exist");
        assert_eq!(cell.symbol(), " ");
    }

    #[rstest::rstest]
    fn render_user_entry() {
        // Given a ChatLogElement with a user entry "hello".
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::user("hello"));
            s
        };

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the bottom row has ">" and the text is bold.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((0, 9)).expect("cell should exist");
        assert_eq!(cell.symbol(), ">");
        assert!(cell.style().add_modifier.contains(Modifier::BOLD));
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

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the text is dark gray on the bottom row.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((0, 9)).expect("cell should exist");
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

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the text starts with "[" (from "[actor]") on the bottom row and is yellow.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((0, 9)).expect("cell should exist");
        assert_eq!(cell.symbol(), "[");
        assert_eq!(cell.style().fg, Some(Color::Yellow));
    }

    #[rstest::rstest]
    fn render_assistant_entry() {
        // Given a ChatLogElement with an assistant entry "hello world".
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut()
                .push_entry(ChatEntry::assistant("hello world"));
            s
        };

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the bottom row has "\u{2726}" (✦) and is cyan.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((0, 9)).expect("cell should exist");
        assert_eq!(cell.symbol(), "\u{2726}");
        assert_eq!(cell.style().fg, Some(Color::Cyan));
    }

    #[rstest::rstest]
    fn render_mixed_entries() {
        // Given a ChatLogElement with system, user, actor, and assistant entries.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut()
                .push_entry(ChatEntry::system("welcome"));
            s.active_session_mut().push_entry(ChatEntry::user("hello"));
            s.active_session_mut()
                .push_entry(ChatEntry::actor("echo", "HELLO"));
            s.active_session_mut()
                .push_entry(ChatEntry::assistant("world"));
            s
        };

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then line 6 is system (dark gray).
        let buffer = terminal.backend().buffer().clone();
        let line6_cell = buffer.cell((0, 6)).expect("cell should exist");
        assert_eq!(line6_cell.symbol(), "w");
        assert_eq!(line6_cell.style().fg, Some(Color::DarkGray));

        // And line 7 is user (">" prefix, bold).
        let line7_cell = buffer.cell((0, 7)).expect("cell should exist");
        assert_eq!(line7_cell.symbol(), ">");
        assert!(line7_cell.style().add_modifier.contains(Modifier::BOLD));

        // And line 8 is actor (yellow, "[" from "[actor]").
        let line8_cell = buffer.cell((0, 8)).expect("cell should exist");
        assert_eq!(line8_cell.symbol(), "[");
        assert_eq!(line8_cell.style().fg, Some(Color::Yellow));

        // And line 9 is assistant (cyan, "\u{2726}" prefix).
        let line9_cell = buffer.cell((0, 9)).expect("cell should exist");
        assert_eq!(line9_cell.symbol(), "\u{2726}");
        assert_eq!(line9_cell.style().fg, Some(Color::Cyan));
    }

    #[rstest::rstest]
    fn render_user_first_line_has_prefix() {
        // Given a ChatLogElement with a user entry containing "hello\nworld".
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut()
                .push_entry(ChatEntry::user("hello\nworld"));
            s
        };

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then line 8 has "> " prefix (bold).
        let buffer = terminal.backend().buffer().clone();
        let line8 = buffer.cell((0, 8)).expect("cell should exist");
        assert_eq!(line8.symbol(), ">");
        assert!(line8.style().add_modifier.contains(Modifier::BOLD));
    }

    #[rstest::rstest]
    fn render_user_continuation_has_no_prefix() {
        // Given a ChatLogElement with a user entry containing "hello\nworld".
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut()
                .push_entry(ChatEntry::user("hello\nworld"));
            s
        };

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // And line 9 has "world" (no prefix, bold).
        let buffer = terminal.backend().buffer().clone();
        let w_cell = buffer.cell((0, 9)).expect("cell should exist");
        assert_eq!(w_cell.symbol(), "w");
        assert!(w_cell.style().add_modifier.contains(Modifier::BOLD));
    }

    #[rstest::rstest]
    fn render_assistant_first_line_has_prefix() {
        // Given a ChatLogElement with an assistant entry containing "line1\nline2".
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut()
                .push_entry(ChatEntry::assistant("line1\nline2"));
            s
        };

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then line 8 has "✦ " prefix (cyan).
        let buffer = terminal.backend().buffer().clone();
        let line8 = buffer.cell((0, 8)).expect("cell should exist");
        assert_eq!(line8.symbol(), "\u{2726}");
        assert_eq!(line8.style().fg, Some(Color::Cyan));
    }

    #[rstest::rstest]
    fn render_assistant_continuation_has_no_prefix() {
        // Given a ChatLogElement with an assistant entry containing "line1\nline2".
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut()
                .push_entry(ChatEntry::assistant("line1\nline2"));
            s
        };

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // And line 9 has "line2" (no prefix, cyan).
        let buffer = terminal.backend().buffer().clone();
        let l_cell = buffer.cell((0, 9)).expect("cell should exist");
        assert_eq!(l_cell.symbol(), "l");
        assert_eq!(l_cell.style().fg, Some(Color::Cyan));
    }

    #[rstest::rstest]
    fn render_first_line_has_prefix() {
        // Given a user entry "a\n\nb".
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::user("a\n\nb"));
            s
        };

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then line 7 is "> a" (bold).
        let buffer = terminal.backend().buffer().clone();
        let line7 = buffer.cell((2, 7)).expect("cell should exist");
        assert_eq!(line7.symbol(), "a");
    }

    #[rstest::rstest]
    fn render_empty_line_between_newlines() {
        // Given a user entry "a\n\nb".
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::user("a\n\nb"));
            s
        };

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then line 8 is empty (middle line between newlines).
        // Verified by checking that line 9 has "b".
        let buffer = terminal.backend().buffer().clone();
        // Line 8 should not have a prefix character (not '>' or 'a' or 'b')
        let line8 = buffer.cell((0, 8)).expect("cell should exist");
        assert_ne!(line8.symbol(), ">");
        assert_ne!(line8.symbol(), "a");
        assert_ne!(line8.symbol(), "b");
    }

    #[rstest::rstest]
    fn render_continuation_has_no_prefix() {
        // Given a user entry "a\n\nb".
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::user("a\n\nb"));
            s
        };

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // And line 9 is "b" (no prefix, bold).
        let buffer = terminal.backend().buffer().clone();
        let line9 = buffer.cell((0, 9)).expect("cell should exist");
        assert_eq!(line9.symbol(), "b");
        assert!(line9.style().add_modifier.contains(Modifier::BOLD));
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

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the top rows are empty and the message appears at the bottom.
        let buffer = terminal.backend().buffer().clone();

        // Top row is empty.
        let top_cell = buffer.cell((0, 0)).expect("cell should exist");
        assert_eq!(top_cell.symbol(), " ");

        // Bottom row has the user message.
        let bottom_cell = buffer.cell((0, 9)).expect("cell should exist");
        assert_eq!(bottom_cell.symbol(), ">");
        assert!(bottom_cell.style().add_modifier.contains(Modifier::BOLD));
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
    fn selected_entry_has_reversed_indicator() {
        // Given a ChatLogElement with 2 entries, first selected.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::user("hello"));
            s.active_session_mut().push_entry(ChatEntry::user("world"));
            s.active_session_mut().select_next_entry(); // selects index 0
            s
        };

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the selected entry has a REVERSED style indicator on the first line.
        let buffer = terminal.backend().buffer().clone();
        // Line 8 has the selected entry (first entry, with ▶ prefix and REVERSED).
        let cell = buffer.cell((0, 8)).expect("cell should exist");
        assert_eq!(cell.symbol(), "\u{25b6}");
        assert!(cell.style().add_modifier.contains(Modifier::REVERSED));
    }

    #[rstest::rstest]
    fn unselected_entry_has_normal_indicator() {
        // Given a ChatLogElement with 2 entries, first selected.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::user("hello"));
            s.active_session_mut().push_entry(ChatEntry::user("world"));
            s.active_session_mut().select_next_entry(); // selects index 0
            s
        };

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Line 9 has the unselected entry (no ▶ prefix, no REVERSED).
        let buffer = terminal.backend().buffer().clone();
        let unselected = buffer.cell((0, 9)).expect("cell should exist");
        assert_eq!(unselected.symbol(), ">");
        assert!(!unselected.style().add_modifier.contains(Modifier::REVERSED));
    }

    #[rstest::rstest]
    fn render_no_selection_has_no_highlight() {
        // Given a ChatLogElement with entries but no selection.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut().push_entry(ChatEntry::user("hello"));
            s.active_session_mut().push_entry(ChatEntry::user("world"));
            s
        };

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then no entries have the REVERSED style or ▶ prefix.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((0, 8)).expect("cell should exist");
        assert_eq!(cell.symbol(), ">");
        assert!(!cell.style().add_modifier.contains(Modifier::REVERSED));
    }

    #[rstest::rstest]
    fn render_pinned_entry_shows_pin_icon() {
        // Given a ChatLogElement with one pinned user entry.
        let mut element = ChatLogElement;
        let state = {
            let mut s = AppState::default();
            s.active_session_mut()
                .push_entry(ChatEntry::user("hello").with_pin(nullslop_protocol::PinPosition::Top));
            s
        };

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the pinned entry's line contains the 📌 character.
        let buffer = terminal.backend().buffer().clone();
        let has_pin = (0..10).any(|row| {
            (0..40).any(|col| {
                buffer
                    .cell((col, row))
                    .is_some_and(|c| c.symbol() == "\u{1F4CC}")
            })
        });
        assert!(has_pin, "pinned entry should show \u{1F4CC} pin icon");
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

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 10);

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
            // Scroll to bottom (auto-scroll).
            // The scroll_offset is None (auto-scroll to bottom).
            s
        };

        let backend = TestBackend::new(40, 5); // 5-line viewport
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 5);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the selected entry (with \u{25b6} prefix) should be visible somewhere
        // in the viewport (not scrolled off the top).
        let buffer = terminal.backend().buffer().clone();
        let has_indicator = (0..5).any(|row| {
            buffer
                .cell((0, row))
                .is_some_and(|c| c.symbol() == "\u{25b6}")
        });
        assert!(
            has_indicator,
            "selected entry should be visible in viewport when scroll-to-selected is active"
        );
    }
}
