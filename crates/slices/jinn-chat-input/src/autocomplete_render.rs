//! Autocomplete popup rendering - renders the prompt template and slash command autocomplete overlay.

use crate::AutocompleteTrigger;
use jinn_kernel::AppState;
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
    // Snapshot what the popup draws so the input facade lock is never held
    // across rendering. Autocomplete matches are already owned values.
    let Some((matches, selected_index, trigger, token_visual)) = state.active_session().with_input(
        |i| {
            let ac = i.autocomplete().clone()?;
            Some((
                ac.matches().to_vec(),
                ac.selected_index(),
                ac.trigger(),
                i.autocomplete_token_visual_row_col(),
            ))
        },
        || None,
    ) else {
        return;
    };

    // `@` popup: matches come from `frontend.file_picker`, not `ac.matches()`.
    if matches!(trigger, AutocompleteTrigger::At | AutocompleteTrigger::AtAt) {
        render_at_popup(frame, input_area, state, selected_index);
        return;
    }

    let Some((token_row, token_col)) = token_visual else {
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

    let no_matches_text = match trigger {
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
    let raw_popup_height: u16 = if matches.is_empty() {
        3 // border top + "no matches" + border bottom
    } else {
        u16::try_from(visible_count + 2).unwrap_or(u16::MAX)
    };

    // Position: horizontally anchored at the trigger's wrapped column, vertically
    // floating one row above the trigger's on-screen visual line (the cursor's
    // line) instead of the top of the whole input box.
    let scroll_offset = state.active_session().with_input(
        jinn_chat_input_msg::ChatInputBoxState::scroll_offset,
        Default::default,
    );
    let trigger_screen_y = input_area
        .y
        .saturating_add(token_row.saturating_sub(scroll_offset) as u16);
    let popup_height = clamp_popup_height(raw_popup_height, trigger_screen_y);
    let popup_y = trigger_screen_y.saturating_sub(popup_height);
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

/// Clamps `popup_height` so the popup never extends above terminal row 0 when
/// its bottom is anchored at `bottom_y`. A minimum height (1 inner row + 2
/// borders = 3) is enforced so the popup never collapses entirely; this may
/// overlap the terminal top on very small terminals, matching prior behavior.
fn clamp_popup_height(raw_height: u16, bottom_y: u16) -> u16 {
    let min_height: u16 = 3;
    raw_height.min(bottom_y).max(min_height)
}

/// Text shown while a directory listing is in flight.
const AT_LOADING: &str = "<loading…>";
/// Text shown when a directory listing is empty/unreadable.
const AT_EMPTY: &str = "<empty>";

/// Renders the `@path` file popup from the file-picker cell.
///
/// Dirs render with a trailing `/`; files render plain. While a listing is
/// in flight, shows `<loading…>`; when the listing is empty, shows `<empty>`.
fn render_at_popup(
    frame: &mut Frame<'_>,
    input_area: Rect,
    state: &AppState,
    selected_index: usize,
) {
    let Some((token_row, token_col, filter)) = state.active_session().with_input(
        |i| {
            i.autocomplete().as_ref()?;
            let (token_row, token_col) = i.autocomplete_token_visual_row_col()?;
            Some((
                token_row,
                token_col,
                i.autocomplete_filter().unwrap_or_default(),
            ))
        },
        || None,
    ) else {
        return;
    };
    let Some(picker) = state.frontend.with_file_picker(|picker| picker.clone()) else {
        return;
    };

    // Build the display rows. The `@` popup narrows by the last path segment
    // of the current filter (what the user is typing), so render and confirm
    // share `visible_entries` as the single source of truth.
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
    let raw_popup_height: u16 = if rows.len() <= 1 {
        3 // border top + single line + border bottom
    } else {
        u16::try_from(visible_count + 2).unwrap_or(u16::MAX)
    };

    // Vertically float the popup one row above the trigger's on-screen visual
    // line (the cursor's line), matching the `#`/`/` popup.
    let scroll_offset = state.active_session().with_input(
        jinn_chat_input_msg::ChatInputBoxState::scroll_offset,
        Default::default,
    );
    let trigger_screen_y = input_area
        .y
        .saturating_add(token_row.saturating_sub(scroll_offset) as u16);
    let popup_height = clamp_popup_height(raw_popup_height, trigger_screen_y);
    let popup_y = trigger_screen_y.saturating_sub(popup_height);
    let popup_x = anchor_x.min(term_width.saturating_sub(popup_width));
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let loading = picker.loading;
    let (start, end) = scroll_window(selected_index, rows.len(), inner.height as usize);
    let mut lines: Vec<Line<'_>> = Vec::with_capacity(inner.height as usize);
    for (i, text) in rows.iter().enumerate().skip(start).take(end - start) {
        let style = if loading || i != selected_index {
            Style::default()
        } else {
            Style::default().add_modifier(Modifier::REVERSED)
        };
        lines.push(Line::styled(text.as_str(), style));
    }
    // Pad remaining inner rows so the popup keeps a fixed height regardless of
    // where the scroll window sits (mirrors the `#`/`/` popup).
    lines.resize(inner.height as usize, Line::from(""));
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Compute a scroll window `[start, end)` that keeps `selected` visible.
///
/// When `total <= visible`, returns `(0, total)`. Otherwise, follows the
/// selection: it advances the window one row at a time once the highlight
/// crosses the bottom edge of the previous window, so the selected entry is
/// always in view (the window trails the highlight, it does not re-center).
pub fn scroll_window(selected: usize, total: usize, visible: usize) -> (usize, usize) {
    if total <= visible {
        return (0, total);
    }
    let start = (selected + 1).saturating_sub(visible);
    let end = (start + visible).min(total);
    (start, end)
}
