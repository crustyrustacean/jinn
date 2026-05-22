//! Session preview popup — shows the last few entries of a highlighted session.
//!
//! Rendered as a bordered overlay when the sidebar sessions section is focused.
//! The popup is anchored bottom-right: its right edge aligns with the right
//! edge of the terminal (just left of the sidebar), and its bottom edge sits
//! just above the sessions section. Displays the last 5 entries rendered using
//! the same entry pipeline as the real chat log, truncated to the last 20 lines.
//! A footer at the bottom shows keybinds across two lines and the session's
//! active provider/model.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::common::app_state::{AppState, FocusScope};
use crate::feat::provider_infra::NO_PROVIDER_ID;
use crate::feat::session::chat_session::{ChatSessionState, SessionState};
use crate::feat::theme::Theme;
use crate::feat::ui::chat_log::entry_to_lines;
use crate::feat::ui::chat_log::shared::RenderContext;
use crate::feat::ui::sidebar::sessions::MAX_VISIBLE_SESSIONS;
use crate::feat::ui::sidebar::sessions::state::sorted_open_sessions;

/// Number of history entries to show in the preview.
const PREVIEW_ENTRY_COUNT: usize = 5;
/// Maximum number of rendered lines to display.
const PREVIEW_MAX_LINES: usize = 20;
/// Default max lines for tool entries when no preference is set.
const DEFAULT_TOOL_ENTRY_MAX_LINES: u16 = 6;

/// Computes the sessions section content height from state.
///
/// Mirrors the logic in `SessionsSection::content_height` so the preview
/// can determine where the sessions section starts without needing the
/// section instance.
pub fn sessions_section_content_height(state: &AppState) -> u16 {
    let session_count = state
        .session
        .iter()
        .filter(|(_, s)| s.session_state() == SessionState::Loaded)
        .count() as u16;
    let visible = session_count.min(MAX_VISIBLE_SESSIONS as u16);
    // entries(N).max(1) + footer(1)
    visible.max(1) + 1
}

/// Renders the session preview popup when the sidebar sessions section is focused.
///
/// Checks focus state, resolves the highlighted session, computes the popup
/// rect anchored bottom-right (above the sessions section, right-aligned with
/// the terminal edge), and delegates to [`render_session_preview`].
/// This is the primary entry point for the TUI render loop.
///
/// - `sidebar_rect`: the full sidebar column rect
/// - `frame_area`: the total frame area (used for right-edge alignment)
pub fn render_session_preview_for_state(
    frame: &mut Frame<'_>,
    sidebar_rect: Rect,
    frame_area: Rect,
    state: &AppState,
) {
    if !matches!(
        state.frontend.scope_stack.current(),
        FocusScope::SidebarSessions
    ) {
        return;
    }
    let Some(idx) = state.frontend.sessions_section.selected_index else {
        return;
    };

    let entries = sorted_open_sessions(state);
    let Some(entry) = entries.get(idx) else {
        return;
    };
    let Some(session) = state.session.get(&entry.id) else {
        return;
    };

    let theme = &state.frontend.theme;
    let tool_max = state.frontend.preferences.tool_entry_max_lines;

    // Compute the sessions section top Y (it's the last section, bottom-anchored).
    let sessions_height = sessions_section_content_height(state);
    let sessions_top_y = sidebar_rect.y + sidebar_rect.height.saturating_sub(sessions_height);

    // Compute content line count for height estimation.
    let inner_width = {
        let popup_width = preview_width(frame_area);
        popup_width.saturating_sub(2)
    };
    let content_lines = build_preview_lines(session, inner_width.max(1), theme, tool_max);
    let line_count = content_lines.len();

    let popup_rect = session_preview_popup_rect(frame_area, sessions_top_y, line_count);

    render_session_preview(frame, popup_rect, session, theme, tool_max);
}

/// Computes the popup width: 60% of frame area, min 30, max frame width.
fn preview_width(frame_area: Rect) -> u16 {
    let w = (f32::from(frame_area.width) * 0.6).ceil() as u16;
    w.max(30).min(frame_area.width)
}

/// Renders the session preview popup into the given frame area.
///
/// Shows a bordered popup with:
/// - Session title in the top border
/// - Up to 20 lines of content from the last 5 entries
/// - A footer with two lines of keybinds and a provider/model status line
pub fn render_session_preview(
    frame: &mut Frame<'_>,
    popup_area: Rect,
    session: &ChatSessionState,
    theme: &Theme,
    tool_entry_max_lines: Option<u16>,
) {
    let inner_width = popup_area.width.saturating_sub(2);
    if inner_width == 0 {
        return;
    }

    let title = session.title().unwrap_or("Untitled Session");

    // Collect the last 5 entries and render them.
    let content_lines = build_preview_lines(session, inner_width, theme, tool_entry_max_lines);

    // Footer: 2 keybinds lines + 1 model line.
    let footer_height = 3u16;
    let content_area_height = popup_area
        .height
        .saturating_sub(2) // borders
        .saturating_sub(footer_height);

    // Clear the popup area.
    frame.render_widget(Clear, popup_area);

    // Render the bordered block with session title.
    let block = Block::default()
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(theme.popup_title),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_unfocused));
    frame.render_widget(block, popup_area);

    // Inner area (inside borders).
    let inner_area = Rect {
        x: popup_area.x + 1,
        y: popup_area.y + 1,
        width: inner_width,
        height: popup_area.height.saturating_sub(2),
    };
    if inner_area.width == 0 || inner_area.height == 0 {
        return;
    }

    // Content paragraph.
    if content_area_height > 0 && !content_lines.is_empty() {
        let content_para = Paragraph::new(content_lines).wrap(Wrap { trim: false });
        let content_area = Rect {
            x: inner_area.x,
            y: inner_area.y,
            width: inner_area.width,
            height: content_area_height.min(inner_area.height),
        };
        frame.render_widget(content_para, content_area);
    }

    // Footer: keybinds + model line at the bottom of the inner area.
    render_keybinds_bar(frame, inner_area, theme);
    render_model_line(frame, inner_area, session, theme);
}

/// Renders the keybinds bar at the bottom of the popup.
///
/// Two lines:
/// - Line 1: `x close · a archive · i insert`
/// - Line 2: `c continue · r rename`
fn render_keybinds_bar(frame: &mut Frame<'_>, inner_area: Rect, theme: &Theme) {
    let key_style = Style::default()
        .fg(theme.accent_action)
        .add_modifier(Modifier::BOLD);
    let sep_style = Style::default().fg(theme.muted_text);

    // Line 1: x close · a archive · i insert
    let line1_y = inner_area.y + inner_area.height.saturating_sub(3);
    let line1_spans = vec![
        Span::styled("x", key_style),
        Span::styled(" close", sep_style),
        Span::styled(" · ", sep_style),
        Span::styled("a", key_style),
        Span::styled(" archive", sep_style),
        Span::styled(" · ", sep_style),
        Span::styled("i", key_style),
        Span::styled(" insert", sep_style),
    ];
    let line1_area = Rect {
        x: inner_area.x,
        y: line1_y,
        width: inner_area.width,
        height: 1,
    };
    frame.render_widget(Paragraph::new(Line::from(line1_spans)), line1_area);

    // Line 2: c continue · r rename
    let line2_y = inner_area.y + inner_area.height.saturating_sub(2);
    let line2_spans = vec![
        Span::styled("c", key_style),
        Span::styled(" continue", sep_style),
        Span::styled(" · ", sep_style),
        Span::styled("r", key_style),
        Span::styled(" rename", sep_style),
    ];
    let line2_area = Rect {
        x: inner_area.x,
        y: line2_y,
        width: inner_area.width,
        height: 1,
    };
    frame.render_widget(Paragraph::new(Line::from(line2_spans)), line2_area);
}

/// Renders the provider/model status line at the very bottom of the popup.
///
/// Shows the model in the same format as the main status bar:
/// `({provider})/{model}` or `"no model selected"` when unset.
/// Right-aligned using `muted_text` style.
fn render_model_line(
    frame: &mut Frame<'_>,
    inner_area: Rect,
    session: &ChatSessionState,
    theme: &Theme,
) {
    let line_y = inner_area.y + inner_area.height.saturating_sub(1);
    let line_area = Rect {
        x: inner_area.x,
        y: line_y,
        width: inner_area.width,
        height: 1,
    };

    let model = session.model();
    let display = if model == NO_PROVIDER_ID {
        "no model selected".to_owned()
    } else if let Some((provider, model_suffix)) = model.split_once('/') {
        format!("({provider})/{model_suffix}")
    } else {
        model.to_owned()
    };

    let style = Style::default().fg(theme.muted_text);
    let line = Paragraph::new(Line::from(Span::styled(display, style))).alignment(Alignment::Right);
    frame.render_widget(line, line_area);
}

/// Builds the preview content lines from the session's last entries.
///
/// Takes the last `PREVIEW_ENTRY_COUNT` entries, renders each via
/// [`entry_to_lines`], flattens, and truncates to `PREVIEW_MAX_LINES`.
fn build_preview_lines(
    session: &ChatSessionState,
    content_width: u16,
    theme: &Theme,
    tool_entry_max_lines: Option<u16>,
) -> Vec<Line<'static>> {
    let history = session.history();
    if history.is_empty() {
        return Vec::new();
    }

    let start = history.len().saturating_sub(PREVIEW_ENTRY_COUNT);
    let entries = &history[start..];

    let ctx = RenderContext {
        content_width,
        _is_selected: false,
        is_expanded: false,
        tool_entry_max_lines: tool_entry_max_lines.unwrap_or(DEFAULT_TOOL_ENTRY_MAX_LINES),
        theme: theme.clone(),
        paired_status: None,
    };

    let mut all_lines = Vec::new();
    for entry in entries {
        all_lines.extend(entry_to_lines(entry, &ctx));
    }

    // Take the last PREVIEW_MAX_LINES lines.
    if all_lines.len() <= PREVIEW_MAX_LINES {
        all_lines
    } else {
        let skip = all_lines.len() - PREVIEW_MAX_LINES;
        all_lines.into_iter().skip(skip).collect()
    }
}

/// Computes the popup rectangle for the session preview overlay.
///
/// The popup is anchored to the right edge of the frame and sits just above
/// the sessions section. Width is 60% of the frame. Height is computed from
/// the content line count plus borders and keybinds bar, capped to fit.
pub fn session_preview_popup_rect(
    frame_area: Rect,
    sessions_top_y: u16,
    content_line_count: usize,
) -> Rect {
    let popup_width = preview_width(frame_area);

    // Total height: content + footer (3) + top border (1) + bottom border (1).
    let desired_height = (content_line_count + 3 + 2) as u16;
    // Cap to available space above the sessions section (with 1-row gap).
    let max_height = sessions_top_y
        .saturating_sub(frame_area.y)
        .saturating_sub(1);
    let popup_height = desired_height.min(max_height).max(5);

    // Right-align: right edge = frame right edge.
    let popup_x = frame_area.x + frame_area.width.saturating_sub(popup_width);
    // Bottom edge = sessions_top_y + 1 (1-row gap above sessions section).
    let popup_y = sessions_top_y.saturating_sub(popup_height + 1);

    Rect::new(popup_x, popup_y, popup_width, popup_height)
}
