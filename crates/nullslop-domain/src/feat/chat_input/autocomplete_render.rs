//! Autocomplete popup rendering — renders the prompt template autocomplete overlay.

use crate::common::app_state::AppState;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Maximum number of visible rows in the autocomplete popup.
pub const AUTOCOMPLETE_MAX_VISIBLE: usize = 20;
/// Minimum popup width.
pub const AUTOCOMPLETE_MIN_WIDTH: u16 = 20;
/// Maximum popup width as fraction of terminal width.
pub const AUTOCOMPLETE_MAX_WIDTH_FRAC: f32 = 0.60;
/// Separator between name and description.
pub const AUTOCOMPLETE_NAME_DESC_SEP: &str = " — ";
/// Text shown when no matches found.
pub const AUTOCOMPLETE_NO_MATCHES: &str = "<no prompts found>";

/// Renders the autocomplete popup overlay above the input box.
///
/// The popup is a transient visual element — not a `UiElement`. It reads autocomplete
/// state directly from `AppState` and renders a bordered box with match entries.
/// The popup is horizontally anchored at the `$` token's screen column and sits
/// directly above the input box.
pub fn render_autocomplete_popup(frame: &mut Frame<'_>, input_area: Rect, state: &AppState) {
    let input = state.active_chat_input();
    let Some(ac) = input.autocomplete().as_ref() else {
        return;
    };

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

    let content_width: u16 = if matches.is_empty() {
        AUTOCOMPLETE_NO_MATCHES
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

    // Position: above the input box, horizontally anchored at the $.
    let popup_y = input_area.y.saturating_sub(popup_height);
    let popup_x = anchor_x.min(term_width.saturating_sub(popup_width));

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Render bordered block.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Render content.
    if matches.is_empty() {
        let line = Line::styled(
            AUTOCOMPLETE_NO_MATCHES,
            Style::default().fg(Color::DarkGray),
        );
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
#[path = "autocomplete_render_tests.rs"]
mod autocomplete_render_tests;
