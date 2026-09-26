//! Session preview popup - shows the last few entries of a highlighted session.
//!
//! Rendered as a bordered overlay when the sidebar sessions section is focused.
//! The popup is anchored bottom-right: its right edge aligns with the right
//! edge of the terminal (just left of the sidebar), and its bottom edge sits
//! just above the sessions section. Displays the last 5 entries rendered using
//! the same entry pipeline as the real chat log, truncated to the last 20 lines.
//! A footer at the bottom shows keybinds across two lines and the session's
//! active cwd and provider/model on the same line.

use jinn_preferences_config::schemas::ChatLogConfig;
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use unicode_segmentation::UnicodeSegmentation;

use crate::sections::sessions::state::sorted_open_sessions;
use jinn_chat_log_view::chat_log::RenderContext;
use jinn_chat_log_view_msg::{PREVIEW_ENTRY_COUNT, PREVIEW_MAX_LINES};
use jinn_domain::common::render_ctx::RenderCtx;
use jinn_domain::feat::ui::chat_log::render_preview;
use jinn_session_state::ChatSessionState;
use jinn_sidebar_msg::SessionPreviewCache;
use jinn_theme::Theme;

/// Default max lines for tool entries when no preference is set.
pub(crate) const DEFAULT_TOOL_ENTRY_MAX_LINES: u16 = 6;
/// Rows between the popup and the cursor row it describes.
///
/// Two rows leaves a one-row gap, so the popup reads as a separate surface
/// rather than colliding with the highlighted session row.
const POPUP_GAP: u16 = 2;

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
    ctx: &RenderCtx,
) {
    let state = ctx.state;
    if state.frontend.sidebar_section() != Some(jinn_sidebar_msg::SidebarSectionId::Sessions) {
        return;
    }
    let Some(idx) = state
        .frontend
        .with_sections(|s| s.sessions.selected_index, || None)
    else {
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
    let tool_max = ctx.config.read::<ChatLogConfig>().tool_entry_max_lines;

    // Anchor the popup to the cursor through the same document layout the
    // sidebar renders with, so it stays attached while the column scrolls.
    let cursor_y = crate::sections::layout::frame_row_of(
        sidebar_rect,
        state,
        ctx.config,
        jinn_sidebar_msg::SidebarSectionId::Sessions,
        u16::try_from(idx).unwrap_or(u16::MAX),
    );

    // Compute content line count for height estimation.
    let mut cache = state.frontend.caches.session_preview_cache.write();
    let inner_width = {
        let popup_width = preview_width(frame_area);
        popup_width.saturating_sub(2)
    };
    let content_lines =
        build_preview_lines(session, inner_width.max(1), theme, tool_max, &mut cache);
    let line_count = content_lines.len();

    let popup_rect = session_preview_popup_rect(frame_area, cursor_y, line_count);

    render_session_preview(frame, popup_rect, session, theme, tool_max, &mut cache);
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
    cache: &mut SessionPreviewCache,
) {
    let inner_width = popup_area.width.saturating_sub(2);
    if inner_width == 0 {
        return;
    }

    let title = session.title().unwrap_or("Untitled Session");

    // Collect the last 5 entries and render them.
    let content_lines =
        build_preview_lines(session, inner_width, theme, tool_entry_max_lines, cache);

    // Footer: 2 keybinds lines + 1 model line.
    let footer_height = 3u16;
    let content_area_height = popup_area
        .height
        .saturating_sub(2) // borders
        .saturating_sub(footer_height);

    // Clear the popup area.
    frame.render_widget(Clear, popup_area);

    // Render the bordered block with session title.
    let block = {
        // Completion badge: `{completed}/{total} · {pct}%`, right-aligned in the
        // top border. Suppressed when the task list has no tasks.
        let badge = {
            let (completed, total) = session.task_list().completion_counts();
            (total > 0).then(|| {
                let pct = completed * 100 / total;
                format!(" {completed}/{total} \u{00B7} {pct}% ")
            })
        };
        let mut block = Block::default()
            .title(Span::styled(
                format!(" {title} "),
                Style::default().fg(theme.popup_title),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border_unfocused));
        if let Some(badge) = badge {
            block = block.title(
                Line::from(Span::styled(badge, Style::default().fg(theme.streaming)))
                    .alignment(Alignment::Right),
            );
        }
        block
    };
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

/// Renders the cwd and provider/model status line at the very bottom of the popup.
///
/// Shows the cwd left-aligned and the model right-aligned on the same line.
/// Long cwd paths are left-truncated with a `…` prefix to fit available space.
/// Model display uses the same format as the main status bar:
/// `({provider})/{model}` or `no model selected` when unset.
/// Both use `muted_text` style.
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

    let model = session.model_selection();
    let model_display = if model.is_no_provider() {
        "no model selected".to_owned()
    } else if let Some(single) = model.as_single() {
        if let Some((provider, model_suffix)) = single.split_once('/') {
            format!("({provider})/{model_suffix}")
        } else {
            single.to_owned()
        }
    } else {
        // Alloy — show "alloy (N models)"
        model.as_alloy().map_or("alloy".to_owned(), |a| {
            format!("alloy ({} models)", a.models.len())
        })
    };

    let cwd_raw = session.cwd().to_string_lossy();
    let model_len = UnicodeSegmentation::graphemes(model_display.as_str(), true).count();
    let available = usize::from(inner_area.width);
    let min_gap = 2;

    let cwd_display = {
        let max_cwd_len = available.saturating_sub(model_len).saturating_sub(min_gap);
        if max_cwd_len == 0 {
            String::new()
        } else {
            let cwd_graphemes: Vec<&str> =
                UnicodeSegmentation::graphemes(cwd_raw.as_ref(), true).collect();
            if cwd_graphemes.len() <= max_cwd_len {
                cwd_raw.into_owned()
            } else {
                let take = max_cwd_len - 1; // 1 for the '…' prefix
                let truncated: String = cwd_graphemes
                    .iter()
                    .rev()
                    .take(take)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .copied()
                    .rev()
                    .collect();
                format!("\u{2026}{truncated}")
            }
        }
    };

    let style = Style::default().fg(theme.muted_text);
    let cwd_len = UnicodeSegmentation::graphemes(cwd_display.as_str(), true).count();
    let padding_len = available.saturating_sub(cwd_len).saturating_sub(model_len);
    let padding = " ".repeat(padding_len);

    let spans = vec![
        Span::styled(cwd_display, style),
        Span::styled(padding, style),
        Span::styled(model_display, style),
    ];
    frame.render_widget(Paragraph::new(Line::from(spans)), line_area);
}

/// Builds the preview content lines for a session, hitting the cache first.
///
/// The rendering itself is not this module's business — it is the chat log's
/// `entry_to_lines` arithmetic, shared with the layout worker so the two cannot
/// drift. What lives here is the cache lookup and the popup's own idea of how
/// wide a preview renders.
fn build_preview_lines(
    session: &ChatSessionState,
    content_width: u16,
    theme: &Theme,
    tool_entry_max_lines: Option<u16>,
    cache: &mut SessionPreviewCache,
) -> Vec<Line<'static>> {
    let history = session.history();
    if history.is_empty() {
        return Vec::new();
    }

    let history_len = history.len();

    // Check cache: hit if session + history length + width all match.
    if let Some(lines) = cache.get(session.session_id(), history_len, content_width) {
        return lines.clone();
    }

    // Cache miss - render.
    let lines = {
        let render_ctx = RenderContext {
            content_width,
            is_selected: false,
            is_expanded: false,
            tool_entry_max_lines: tool_entry_max_lines.unwrap_or(DEFAULT_TOOL_ENTRY_MAX_LINES),
            theme: theme.clone(),
            paired_status: None,
            is_streaming: false,
            is_waiting_on_subagent: false,
        };
        render_preview(history, &render_ctx, PREVIEW_ENTRY_COUNT, PREVIEW_MAX_LINES)
    };

    // Store in cache.
    cache.insert(
        session.session_id().clone(),
        history_len,
        content_width,
        lines.clone(),
    );

    lines
}

/// Computes the popup rectangle for the session preview overlay.
///
/// The popup is anchored to the right edge of the frame and sits just above
/// the cursor row in the sessions section, with a 1-row gap. Width is 60% of
/// the frame. Height is computed from the content line count plus borders and
/// keybinds bar, capped to fit within the available space above the cursor.
pub fn session_preview_popup_rect(
    frame_area: Rect,
    cursor_y: u16,
    content_line_count: usize,
) -> Rect {
    let popup_width = preview_width(frame_area);

    // Total height: content + footer (3) + top border (1) + bottom border (1).
    let desired_height = (content_line_count + 3 + 2) as u16;
    // Cap to available space above the cursor (with 1-row gap).
    let max_height = cursor_y
        .saturating_sub(frame_area.y)
        .saturating_sub(POPUP_GAP);
    let popup_height = desired_height.min(max_height).max(5);

    // Right-align: right edge = frame right edge.
    let popup_x = frame_area.x + frame_area.width.saturating_sub(popup_width);
    // Bottom edge sits 1 row above the cursor.
    let popup_y = cursor_y
        .saturating_sub(popup_height)
        .saturating_sub(POPUP_GAP);

    Rect::new(popup_x, popup_y, popup_width, popup_height)
}
