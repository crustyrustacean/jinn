//! Renders the conversation history.
//!
//! Each entry in the chat log is displayed with a distinct visual style so the user
//! can tell them apart at a glance:
//!
//! - **User messages** appear bold with a `>` prefix.
//! - **System messages** appear muted with indentation.
//! - **Actor messages** appear highlighted with the actor's name and content.
//! - **Assistant messages** appear in cyan (no icon — color distinguishes them).
//!
//! Text wraps within the available space.

use crate::common::ui_element::UiElement;
use crate::protocol::{ChatEntryKind, TableData};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::common::app_state::AppState;

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
fn entry_to_lines(entry: &crate::protocol::ChatEntry, is_selected: bool) -> Vec<Line<'static>> {
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
        ChatEntryKind::Error(text) => {
            let prefix = if pinned { "📌   " } else { "  " };
            multiline_styled(
                text,
                prefix,
                "  ",
                Style::default().fg(Color::Red),
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
            let prefix = if pinned { "📌 " } else { "" };
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
        ChatEntryKind::Table(data) => {
            let prefix = if pinned { "📌 " } else { "  " };
            table_to_lines(data, prefix, is_selected)
        }
        ChatEntryKind::Thinking(text) => {
            let prefix = if pinned { "📌   " } else { "  " };
            multiline_styled(
                text,
                prefix,
                "  ",
                Style::default().fg(Color::DarkGray),
                is_selected,
            )
        }
    }
}

/// Split text on `\n` and produce styled lines with the given prefix/indent.
///
/// Render a [`TableData`] as aligned, styled lines.
///
/// Builds column widths from headers and rows, then produces:
/// - A bold header line
/// - A separator line
/// - Styled data rows with per-cell coloring
fn table_to_lines(data: &TableData, prefix: &str, is_selected: bool) -> Vec<Line<'static>> {
    let prefix = prefix.to_owned();
    let num_cols = data.headers.len();
    if num_cols == 0 {
        return vec![Line::from(Span::styled(
            format!("{prefix}(empty table)"),
            Style::default().fg(Color::DarkGray),
        ))];
    }

    // Compute column widths: max of header and all row cells.
    let mut col_widths = vec![0usize; num_cols];
    for (i, h) in data.headers.iter().enumerate() {
        col_widths[i] = col_widths[i].max(unicode_segementation_display_width(&h.content));
    }
    for row in &data.rows {
        for (i, cell) in row.iter().enumerate() {
            if i < num_cols {
                col_widths[i] =
                    col_widths[i].max(unicode_segementation_display_width(&cell.content));
            }
        }
    }

    let sep = " │ ";
    let mut lines = Vec::new();

    // Header line.
    let header_spans = build_row_spans(
        &data.headers,
        &col_widths,
        sep,
        Style::default().add_modifier(Modifier::BOLD),
    );
    let header_line = if is_selected {
        let mut spans = vec![Span::styled(
            format!("▶ {prefix}"),
            Style::default().add_modifier(Modifier::REVERSED),
        )];
        spans.extend(header_spans);
        Line::from(spans)
    } else {
        let mut spans = vec![Span::raw(prefix.clone())];
        spans.extend(header_spans);
        Line::from(spans)
    };
    lines.push(header_line);

    // Separator line.
    let sep_parts: Vec<String> = col_widths.iter().map(|&w| "─".repeat(w)).collect();
    let sep_text = format!("{prefix}{}", sep_parts.join("─┼─"));
    lines.push(Line::from(Span::styled(
        sep_text,
        Style::default().fg(Color::DarkGray),
    )));

    // Data rows.
    for row in &data.rows {
        let row_spans = build_row_spans(row, &col_widths, sep, Style::default());
        let mut spans = vec![Span::raw(prefix.clone())];
        spans.extend(row_spans);
        lines.push(Line::from(spans));
    }

    lines
}

/// Build styled spans for a single table row, padding cells to column width.
fn build_row_spans(
    cells: &[Span<'static>],
    col_widths: &[usize],
    separator: &str,
    default_style: Style,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, cell) in cells.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(separator.to_owned()));
        }
        let width = unicode_segementation_display_width(&cell.content);
        let padding = col_widths
            .get(i)
            .copied()
            .unwrap_or(0)
            .saturating_sub(width);
        // Merge the cell's style with the default style.
        let style = if cell.style == Style::default() {
            default_style
        } else {
            cell.style.patch(default_style)
        };
        spans.push(Span::styled(
            format!("{}{}", cell.content, " ".repeat(padding)),
            style,
        ));
    }
    spans
}

/// Compute the display width of a string using Unicode grapheme clusters.
fn unicode_segementation_display_width(s: &str) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    s.graphemes(true)
        .map(|g| {
            // Emoji and wide characters take 2 columns; everything else takes 1.
            // This is a simplified heuristic — full-width detection would need
            // unicode-width, but for our use case (provider names, counts, status)
            // this is sufficient.
            if g.chars().any(|c| c as u32 > 0x2000) {
                2
            } else {
                1
            }
        })
        .sum()
}

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
    let text = text.trim_start_matches('\n');
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
    use crate::protocol::ChatEntry;
    use nullslop_testutil::setup_term;

    use super::*;
    use crate::common::app_state::AppState;

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
    fn render_user_entry() {
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

        let (mut terminal, area) = setup_term(40, 10);

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

        let (mut terminal, area) = setup_term(40, 10);

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

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the bottom row has the text content (no icon) and is cyan.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((0, 9)).expect("cell should exist");
        assert_eq!(cell.symbol(), "h");
        assert_eq!(cell.style().fg, Some(Color::Cyan));
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

        let (mut terminal, area) = setup_term(40, 10);

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

        let (mut terminal, area) = setup_term(40, 10);

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

        let (mut terminal, area) = setup_term(40, 10);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then line 8 has the text content (no icon) and is cyan.
        let buffer = terminal.backend().buffer().clone();
        let line8 = buffer.cell((0, 8)).expect("cell should exist");
        assert_eq!(line8.symbol(), "l");
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

        let (mut terminal, area) = setup_term(40, 10);

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

        let (mut terminal, area) = setup_term(40, 10);

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

        let (mut terminal, area) = setup_term(40, 10);

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

        let (mut terminal, area) = setup_term(40, 10);

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

        let (mut terminal, area) = setup_term(40, 10);

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

        let (mut terminal, area) = setup_term(40, 10);

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

        let (mut terminal, area) = setup_term(40, 10);

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

        let (mut terminal, area) = setup_term(40, 10);

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
            // Scroll to bottom (auto-scroll).
            // The scroll_offset is None (auto-scroll to bottom).
            s
        };

        let (mut terminal, area) = setup_term(40, 5); // 5-line viewport

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

        // Then the header row contains "Provider" with bold styling.
        let buffer = terminal.backend().buffer().clone();
        // Find a cell that contains "P" (from "Provider") with bold modifier.
        let has_bold_header = (0..10).any(|row| {
            (0..60).any(|col| {
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
            (0..60).any(|col| buffer.cell((col, row)).is_some_and(|c| c.symbol() == "o"))
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
            (0..60).any(|col| {
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

        // Then the text is red on the bottom row.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((0, 9)).expect("cell should exist");
        assert_eq!(cell.symbol(), "C");
        assert_eq!(cell.style().fg, Some(Color::Red));
    }
}
