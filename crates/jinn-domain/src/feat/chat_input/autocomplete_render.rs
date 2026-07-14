//! Autocomplete popup rendering - renders the prompt template and slash command autocomplete overlay.

use crate::common::app_state::AppState;
use crate::feat::chat_input::AutocompleteTrigger;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// Maximum number of visible rows in the autocomplete popup.
pub const AUTOCOMPLETE_MAX_VISIBLE: usize = 20;
/// Minimum popup width.
pub const AUTOCOMPLETE_MIN_WIDTH: u16 = 20;
/// Maximum popup width as fraction of terminal width.
pub const AUTOCOMPLETE_MAX_WIDTH_FRAC: f32 = 0.60;
/// Separator between name and description.
pub const AUTOCOMPLETE_NAME_DESC_SEP: &str = " - ";
/// Text shown when no prompt template matches found.
const NO_PROMPTS_FOUND: &str = "<no prompts found>";
/// Text shown when no slash command matches found.
const NO_COMMANDS_FOUND: &str = "<no commands found>";

/// Renders the autocomplete popup overlay above the input box.
///
/// The popup is a transient visual element - not a `UiElement`. It reads autocomplete
/// state directly from `AppState` and renders a bordered box with match entries.
/// The popup is horizontally anchored at the trigger token's screen column and sits
/// directly above the input box.
pub fn render_autocomplete_popup(frame: &mut Frame<'_>, input_area: Rect, state: &AppState) {
    let input = state.active_chat_input();
    let Some(ac) = input.autocomplete().as_ref() else {
        return;
    };

    // `@` popup: matches come from `frontend.file_picker`, not `ac.matches()`.
    if matches!(
        ac.trigger(),
        AutocompleteTrigger::At | AutocompleteTrigger::AtAt
    ) {
        render_at_popup(frame, input_area, state, ac.selected_index());
        return;
    }

    let matches = ac.matches();
    let selected_index = ac.selected_index();
    let Some(token_col) = input.autocomplete_token_screen_col() else {
        return;
    };

    // Prompt indent is always 2 columns ("> " on first line, "  " on continuation).
    let prompt_indent: u16 = 2;
    let anchor_x = input_area.x + prompt_indent + token_col as u16;

    // Compute popup dimensions.
    let term_width = frame.area().width;
    let max_width = ((f32::from(term_width) * AUTOCOMPLETE_MAX_WIDTH_FRAC).ceil() as u16)
        .max(AUTOCOMPLETE_MIN_WIDTH)
        .min(term_width);

    let no_matches_text = match ac.trigger() {
        AutocompleteTrigger::Hash => NO_PROMPTS_FOUND,
        AutocompleteTrigger::Slash => NO_COMMANDS_FOUND,
        // `@` popup reads `frontend.file_picker`; rendered separately below.
        AutocompleteTrigger::At | AutocompleteTrigger::AtAt => "<empty>",
    };

    let content_width: u16 = if matches.is_empty() {
        no_matches_text
            .len()
            .try_into()
            .unwrap_or(AUTOCOMPLETE_MIN_WIDTH)
    } else {
        matches
            .iter()
            .map(|m| m.name.len() + AUTOCOMPLETE_NAME_DESC_SEP.len() + m.description.len())
            .max()
            .unwrap_or(0)
            .try_into()
            .unwrap_or(AUTOCOMPLETE_MIN_WIDTH)
    };

    // +2 for left and right border columns.
    let popup_width = content_width
        .saturating_add(2)
        .max(AUTOCOMPLETE_MIN_WIDTH)
        .min(max_width);

    let visible_count = matches.len().min(AUTOCOMPLETE_MAX_VISIBLE);
    let popup_height: u16 = if matches.is_empty() {
        3 // border top + "no matches" + border bottom
    } else {
        u16::try_from(visible_count + 2)
            .unwrap_or(u16::MAX)
            .min(input_area.y)
    };

    // Position: above the input box, horizontally anchored at the #.
    let popup_y = input_area.y.saturating_sub(popup_height);
    let popup_x = anchor_x.min(term_width.saturating_sub(popup_width));

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the popup area so content behind it doesn't show through.
    frame.render_widget(Clear, popup_area);

    // Render bordered block.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Render content.
    if matches.is_empty() {
        let line = Line::styled(no_matches_text, Style::default().fg(Color::DarkGray));
        let paragraph = Paragraph::new(line);
        frame.render_widget(paragraph, inner);
    } else {
        let mut lines = Vec::with_capacity(inner.height as usize);
        let (start, end) = scroll_window(selected_index, matches.len(), inner.height as usize);
        for (i, m) in matches.iter().enumerate().skip(start).take(end - start) {
            let text = if m.description.is_empty() {
                m.name.clone()
            } else {
                format!("{}{}{}", m.name, AUTOCOMPLETE_NAME_DESC_SEP, m.description)
            };
            let style = if i == selected_index {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            lines.push(Line::styled(text, style));
        }
        // Pad remaining rows with empty lines.
        while lines.len() < inner.height as usize {
            lines.push(Line::from(""));
        }
        let paragraph = Paragraph::new(lines);
        frame.render_widget(paragraph, inner);
    }
}

/// Text shown while a directory listing is in flight.
const AT_LOADING: &str = "<loading…>";
/// Text shown when a directory listing is empty/unreadable.
const AT_EMPTY: &str = "<empty>";

/// Renders the `@path` file popup from `frontend.file_picker`.
///
/// Dirs render with a trailing `/`; files render plain. While a listing is
/// in flight, shows `<loading…>`; when the listing is empty, shows `<empty>`.
fn render_at_popup(
    frame: &mut Frame<'_>,
    input_area: Rect,
    state: &AppState,
    selected_index: usize,
) {
    let input = state.active_chat_input();
    let Some(ac) = input.autocomplete() else {
        return;
    };
    let Some(token_col) = input.autocomplete_token_screen_col() else {
        return;
    };
    let picker = &state.frontend.file_picker;

    // Build the display rows. The `@` popup narrows by the last path segment
    // of the current filter (what the user is typing), so render and confirm
    // share `visible_entries` as the single source of truth.
    let filter = input.autocomplete_filter().unwrap_or_default();
    let visible = picker.visible_entries(&filter);
    let rows: Vec<String> = if picker.loading {
        vec![AT_LOADING.to_owned()]
    } else if visible.is_empty() {
        vec![AT_EMPTY.to_owned()]
    } else {
        visible
            .iter()
            .map(|e| {
                if e.is_dir {
                    format!("{}/", e.name)
                } else {
                    e.name.clone()
                }
            })
            .collect()
    };

    let prompt_indent: u16 = 2;
    let anchor_x = input_area.x + prompt_indent + token_col as u16;
    let term_width = frame.area().width;
    let max_width = ((f32::from(term_width) * AUTOCOMPLETE_MAX_WIDTH_FRAC).ceil() as u16)
        .max(AUTOCOMPLETE_MIN_WIDTH)
        .min(term_width);

    let content_width: u16 = rows
        .iter()
        .map(|r| r.chars().count())
        .max()
        .unwrap_or(0)
        .try_into()
        .unwrap_or(AUTOCOMPLETE_MIN_WIDTH);
    let popup_width = content_width
        .saturating_add(2)
        .max(AUTOCOMPLETE_MIN_WIDTH)
        .min(max_width);

    let visible_count = rows.len().min(AUTOCOMPLETE_MAX_VISIBLE);
    let popup_height: u16 = if rows.len() <= 1 {
        3 // border top + single line + border bottom
    } else {
        u16::try_from(visible_count + 2)
            .unwrap_or(u16::MAX)
            .min(input_area.y)
    };

    let popup_y = input_area.y.saturating_sub(popup_height);
    let popup_x = anchor_x.min(term_width.saturating_sub(popup_width));
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let loading = picker.loading;
    let lines: Vec<Line<'_>> = rows
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let style = if loading || i != selected_index {
                Style::default()
            } else {
                Style::default().add_modifier(Modifier::REVERSED)
            };
            Line::styled(text.as_str(), style)
        })
        .collect();
    // Silence the unused-ac warning; `ac` only confirms the popup is active.
    let _ = ac;
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Compute a scroll window `[start, end)` that keeps `selected` visible.
///
/// When `total <= visible`, returns `(0, total)`. Otherwise, centers the window
/// around the selected entry so it is always in view.
pub fn scroll_window(selected: usize, total: usize, visible: usize) -> (usize, usize) {
    if total <= visible {
        return (0, total);
    }
    let start = (selected + 1).saturating_sub(visible);
    let end = (start + visible).min(total);
    (start, end)
}

#[cfg(test)]
mod tests {
    use crate::common::app_state::AppState;
    use crate::feat::chat_input::intent::handle_insert_char;
    use crate::feat::file_lister::{FileEntry, FilePickerState};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Renders the `@` popup into a string for assertion.
    fn render_to_string(state: &AppState) -> String {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let input_area = ratatui::layout::Rect::new(0, 20, 80, 4);
        terminal
            .draw(|frame| {
                super::render_autocomplete_popup(frame, input_area, state);
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        buffer
            .content
            .iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect()
    }

    #[test]
    fn render_at_popup_shows_directory_entry_with_trailing_slash() {
        // Given an active @ popup seeded with a directory entry.
        let mut state = AppState::default();
        let _ = handle_insert_char('@', &mut state);
        state.frontend.file_picker = FilePickerState::with_entries(vec![
            FileEntry {
                name: "src".into(),
                is_dir: true,
            },
            FileEntry {
                name: "img.png".into(),
                is_dir: false,
            },
        ]);

        // When rendering.
        let rendered = render_to_string(&state);

        // Then the directory entry shows a trailing slash and the file does not.
        assert!(
            rendered.contains("src/"),
            "dir entry should have trailing slash"
        );
        assert!(
            rendered.contains("img.png"),
            "file entry should render plain"
        );
    }

    #[test]
    fn render_at_popup_shows_loading_state() {
        // Given an active @ popup with a listing in flight.
        let mut state = AppState::default();
        let _ = handle_insert_char('@', &mut state);
        state.frontend.file_picker = FilePickerState {
            loading: true,
            ..FilePickerState::default()
        };

        // When rendering.
        let rendered = render_to_string(&state);

        // Then the loading placeholder is shown.
        assert!(
            rendered.contains("loading"),
            "loading state should be rendered"
        );
    }

    #[test]
    fn render_at_popup_shows_empty_state() {
        // Given an active @ popup with an empty listing (not loading).
        let mut state = AppState::default();
        let _ = handle_insert_char('@', &mut state);
        state.frontend.file_picker = FilePickerState::default(); // empty, not loading

        // When rendering.
        let rendered = render_to_string(&state);

        // Then the empty placeholder is shown.
        assert!(rendered.contains("empty"), "empty state should be rendered");
    }

    #[test]
    fn render_at_popup_narrows_rows_by_typed_prefix() {
        // Given an active @ popup with several entries and a typed filter 'sr'.
        let mut state = AppState::default();
        let _ = handle_insert_char('@', &mut state);
        // Type 'sr' so the last path segment (the filename being typed) is 'sr'.
        let _ = handle_insert_char('s', &mut state);
        let _ = handle_insert_char('r', &mut state);
        state.frontend.file_picker = FilePickerState::with_entries(vec![
            FileEntry {
                name: "src".into(),
                is_dir: true,
            },
            FileEntry {
                name: "srv".into(),
                is_dir: true,
            },
            FileEntry {
                name: "static".into(),
                is_dir: true,
            },
            FileEntry {
                name: "img.png".into(),
                is_dir: false,
            },
        ]);

        // When rendering.
        let rendered = render_to_string(&state);

        // Then only entries starting with 'sr' are shown.
        assert!(rendered.contains("src/"), "src/ should match prefix 'sr'");
        assert!(rendered.contains("srv/"), "srv/ should match prefix 'sr'");
        assert!(
            !rendered.contains("static"),
            "static should be filtered out by prefix 'sr'"
        );
        assert!(
            !rendered.contains("img.png"),
            "img.png should be filtered out by prefix 'sr'"
        );
    }

    #[test]
    fn render_at_popup_shows_empty_when_no_prefix_match() {
        // Given an active @ popup whose entries don't match the typed filter.
        let mut state = AppState::default();
        let _ = handle_insert_char('@', &mut state);
        // Type 'zzz' which matches nothing.
        for ch in "zzz".chars() {
            let _ = handle_insert_char(ch, &mut state);
        }
        state.frontend.file_picker = FilePickerState::with_entries(vec![FileEntry {
            name: "src".into(),
            is_dir: true,
        }]);

        // When rendering.
        let rendered = render_to_string(&state);

        // Then the empty placeholder is shown (distinct from a directory that is
        // actually empty — here the directory has entries but none match).
        assert!(
            rendered.contains("empty"),
            "filtered-out entries should render as <empty>"
        );
    }
}
