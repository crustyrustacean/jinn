//! Layout computation and rendering for the application.

use nullslop_domain::{Mode, PickerKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui_tabs::{TabManager, TabsBar, TabsStyle};
use ratatui_which_key::{PopupPosition, WhichKey};

use crate::TuiApp;
use crate::app::{CHAT_PANE, PaneFocus};
use crate::selection::find_last_nonws_in_row;

/// Minimum terminal width.
pub const MIN_WIDTH: u16 = 40;
/// Minimum terminal height.
pub const MIN_HEIGHT: u16 = 14;

/// Top-level application layout areas.
pub struct AppLayout {
    /// The tab bar area (1 row at top).
    pub tabs: Rect,
    /// The main content area (fills remaining space).
    pub content: Rect,
    /// The streaming indicator area (1 row between content and counter).
    pub indicator: Rect,
    /// The queue display area (dynamic height based on queue length).
    pub queue: Rect,
    /// The character counter area (1 row above input, chat tab only).
    pub counter: Rect,
    /// The input box area (3 rows at bottom, chat tab only).
    pub input: Rect,
    /// The status bar area (1 row at very bottom).
    pub status_bar: Rect,
}

impl AppLayout {
    /// Returns `true` if the given area meets minimum size requirements.
    #[must_use]
    pub const fn meets_min_size(area: Rect) -> bool {
        area.width >= MIN_WIDTH && area.height >= MIN_HEIGHT
    }

    /// Computes the layout for the given terminal area.
    ///
    /// `input_lines` is the number of visual lines the input box needs
    /// (used for dynamic multi-line input height).
    ///
    /// `queue_lines` is the number of rows for the queue display area
    /// (0 when queue is empty).
    ///
    /// `max_input_height` caps the input box height (e.g., 50% of terminal).
    #[must_use]
    pub fn new(area: Rect, input_lines: u16, queue_lines: u16, max_input_height: u16) -> Self {
        let [tabs, rest] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);

        let input_height = (2 + input_lines.max(1)).min(max_input_height);
        let [content, indicator, queue, counter, input, status_bar] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(queue_lines),
            Constraint::Length(1),
            Constraint::Length(input_height),
            Constraint::Length(1),
        ])
        .areas(rest);

        Self {
            tabs,
            content,
            indicator,
            queue,
            counter,
            input,
            status_bar,
        }
    }
}

/// Build the default tab manager with Chat and Dashboard tabs.
pub fn init_tab_manager() -> TabManager {
    let mut mgr = TabManager::new();
    mgr.add_tab("Chat");
    mgr.add_tab("Dashboard");
    mgr
}

/// Renders the full application frame.
pub fn render(app: &mut TuiApp, frame: &mut Frame<'_>) {
    let area = frame.area();
    if !AppLayout::meets_min_size(area) {
        render_too_small(frame, area, app);
        return;
    }

    // Pre-render mutation: set wrap width and scroll offset using a write lock.
    {
        let mut wstate = app.core.state.write();
        let text_width = area.width.saturating_sub(2) as usize;
        wstate.active_chat_input_mut().set_wrap_width(text_width);
        // Need layout to know inner height — compute a preliminary layout.
        let queue_len = wstate.active_session().queue_len() as u16;
        let max_input_height = area.height / 2;
        let pre_layout = AppLayout::new(
            area,
            wstate.active_chat_input().visual_line_count() as u16,
            queue_len,
            max_input_height,
        );
        if wstate.frontend.mode == Mode::Input {
            let inner_height = pre_layout.input.height.saturating_sub(2) as usize;
            wstate.active_chat_input_mut().scroll_to_cursor(inner_height);
        }
    }

    let state = app.core.state.read();

    let queue_len = state.active_session().queue_len() as u16;
    let max_input_height = area.height / 2;
    let layout = AppLayout::new(
        area,
        state.active_chat_input().visual_line_count() as u16,
        queue_len,
        max_input_height,
    );

    // Tab bar — always visible.
    render_tab_bar(frame, layout.tabs, &app.tab_manager);

    // Collect selectable rects during rendering.
    let mut rects = vec![];
    // Split border lines (rendered after elements, before selection highlight).
    let mut borders: Option<Vec<crate::split_borders::BorderLine>> = None;

    match state.frontend.active_tab {
        nullslop_domain::ActiveTab::Chat => {
            let content_area = if app.pinned_pane_visible {
                app.split_manager.set_viewport(layout.content);
                let areas = app.split_manager.areas();
                let result = crate::split_borders::compute_split_borders(areas);
                let chat_rect = result.rect_for(CHAT_PANE).unwrap_or(layout.content);

                if app.pinned_pane_visible {
                    let pinned_rect = app
                        .pinned_pane_id
                        .and_then(|id| result.rect_for(id))
                        .unwrap_or_default();

                    // Render pinned panel into sidebar.
                    if let Some(element) = app.ui_registry.get_mut("pinned-panel") {
                        element.render(frame, pinned_rect, &state);
                        if element.is_selectable() && app.pane_focus == PaneFocus::Pinned {
                            rects.push(pinned_rect);
                        }
                    }
                }

                borders = Some(result.lines);
                chat_rect
            } else {
                layout.content
            };

            // Chat log
            if let Some(element) = app.ui_registry.get_mut("chat-log") {
                element.render(frame, content_area, &state);
                if element.is_selectable() && app.pane_focus == PaneFocus::Chat {
                    rects.push(content_area);
                }
            }
            // Streaming indicator (dedicated row between content and input)
            if let Some(element) = app.ui_registry.get_mut("streaming-indicator") {
                element.render(frame, layout.indicator, &state);
            }
            // Queue display
            if let Some(element) = app.ui_registry.get_mut("queue-display") {
                element.render(frame, layout.queue, &state);
            }
            // Character counter
            if let Some(element) = app.ui_registry.get_mut("char-counter") {
                element.render(frame, layout.counter, &state);
            }
            // Input box
            if let Some(element) = app.ui_registry.get_mut("chat-input-box") {
                element.render(frame, layout.input, &state);
            }

            // Autocomplete popup overlay (transient, not a UiElement).
            if state.active_chat_input().autocomplete().is_some() {
                nullslop_domain::feat::chat_input::autocomplete_render::render_autocomplete_popup(
                    frame,
                    layout.input,
                    &state,
                );
            }
        }
        nullslop_domain::ActiveTab::Dashboard => {
            // Dashboard fills the entire content area
            if let Some(element) = app.ui_registry.get_mut("dashboard") {
                element.render(frame, layout.content, &state);
                if element.is_selectable() {
                    rects.push(layout.content);
                }
            }
        }
    }

    // Status bar — always visible at bottom.
    if let Some(element) = app.ui_registry.get_mut("status-bar") {
        element.render(frame, layout.status_bar, &state);
    }

    // Which-key popup overlay (app-level, not a component element)
    render_which_key(frame, &mut app.which_key);

    if state.frontend.mode == Mode::Picker {
        render_picker(frame, area, &state);
        // Provider picker popup is selectable — not a UiElement, register inline.
        rects.push(nullslop_selection_widget::compute_popup_rect(area));
    }

    // Release the state read lock before clipboard flush needs &mut app.
    drop(state);

    app.selectable_rects.rebuild(rects);

    // Draw split borders on top of element content (but below selection highlight).
    if let Some(ref lines) = borders {
        crate::split_borders::render_borders(frame, lines);
    }

    // Apply selection highlight after all elements have rendered.
    apply_selection_highlight(app, frame.buffer_mut());

    // Flush pending clipboard copy (reads buffer, writes system clipboard).
    flush_pending_clipboard(app, frame.buffer_mut());
}

/// Applies line-based selection highlight to the buffer.
///
/// Row classification:
/// - **Single line** (anchor_y == focus_y): column-based highlight from min(ax,fx) to max(ax,fx).
/// - **First line** (anchor row): from anchor_x to last non-whitespace char.
/// - **Middle lines**: from bounds.x to last non-whitespace char.
/// - **Last line** (focus row): from bounds.x to focus_x.
///
/// For first and middle rows, the highlight extends from start_x to the last
/// non-whitespace character — internal spaces between words are included.
fn apply_selection_highlight(app: &TuiApp, buf: &mut ratatui::buffer::Buffer) {
    let (anchor, focus, bounds) = match app.selection {
        crate::selection::SelectionState::Dragging {
            anchor,
            focus,
            bounds,
        }
        | crate::selection::SelectionState::Active {
            anchor,
            focus,
            bounds,
        } => (anchor, focus, bounds),
        crate::selection::SelectionState::Idle => return,
    };

    let bounds_right = bounds.right().saturating_sub(1);
    let anchor_x = anchor.0.clamp(bounds.x, bounds_right);
    let focus_x = focus.0.clamp(bounds.x, bounds_right);
    let top_y = anchor.1.min(focus.1).max(bounds.y);
    let bot_y = anchor.1.max(focus.1).min(bounds.bottom().saturating_sub(1));

    for y in top_y..=bot_y {
        let (start_x, end_x) = if top_y == bot_y {
            // Single line — column selection.
            (anchor_x.min(focus_x), anchor_x.max(focus_x))
        } else if y == anchor.1 {
            // First line — from anchor_x to last non-whitespace.
            let end = find_last_nonws_in_row(buf, y, anchor_x, bounds_right).unwrap_or(anchor_x);
            (anchor_x, end)
        } else if y == focus.1 {
            // Last line — from bounds.x to focus_x.
            (bounds.x, focus_x)
        } else {
            // Middle line — from bounds.x to last non-whitespace.
            let end = find_last_nonws_in_row(buf, y, bounds.x, bounds_right).unwrap_or(bounds.x);
            (bounds.x, end)
        };

        for x in start_x..=end_x {
            if let Some(cell) = buf.cell_mut((x, y)) {
                let fg = cell.fg;
                let bg = cell.bg;
                if fg == bg {
                    // Swapping identical colors is invisible
                    // (e.g. both Reset — the default for user messages
                    // and empty cells). Use explicit highlight colors.
                    cell.set_fg(Color::Black);
                    cell.set_bg(Color::White);
                } else {
                    cell.set_fg(bg);
                    cell.set_bg(fg);
                }
            }
        }
    }
}

/// If a clipboard copy is pending, extracts the selected text from the buffer
/// and copies it to the system clipboard. Clears the pending flag regardless
/// of success or failure.
///
/// The clipboard write runs on a spawned thread that holds the
/// [`arboard::Clipboard`] open for a few seconds after writing. On X11,
/// clipboard data is only available while the `Clipboard` instance is alive —
/// dropping it immediately prevents clipboard managers from syncing.
fn flush_pending_clipboard(app: &mut TuiApp, buf: &ratatui::buffer::Buffer) {
    if !app.pending_clipboard {
        return;
    }
    app.pending_clipboard = false;

    let text = match app.selection.extract_text(buf) {
        Some(text) if !text.is_empty() => text,
        _ => return,
    };

    // Spawn a thread to hold the clipboard open for clipboard managers.
    std::thread::spawn(move || {
        let mut cb = match arboard::Clipboard::new() {
            Ok(cb) => cb,
            Err(e) => {
                tracing::warn!(err = %e, "failed to create clipboard");
                return;
            }
        };
        if let Err(e) = cb.set_text(&text) {
            tracing::warn!(err = %e, "failed to copy selection to clipboard");
            return;
        }
        tracing::debug!(len = text.len(), "copied selection to clipboard");
        // Hold clipboard open so clipboard managers can sync.
        // cb must live through the sleep — X11 clipboard data is only
        // available while the Clipboard instance is alive.
        std::thread::sleep(std::time::Duration::from_secs(2));
    });
}

/// Renders the tab bar.
fn render_tab_bar(frame: &mut Frame<'_>, area: Rect, manager: &TabManager) {
    let tabs = manager.tabs();
    let active_id = manager.active_id();
    let bar = TabsBar::new(tabs, active_id).tabs_style(TabsStyle {
        active: Style::default().fg(Color::Black).bg(Color::Yellow),
        inactive: Style::default().fg(Color::Gray),
        ..TabsStyle::default()
    });
    frame.render_widget(bar, area);
}

/// Renders the which-key popup overlay.
fn render_which_key(frame: &mut Frame<'_>, state: &mut crate::app::WhichKeyInstance) {
    let widget = WhichKey::new()
        .position(PopupPosition::BottomRight)
        .border_style(Style::default().fg(Color::Yellow));
    let buf = frame.buffer_mut();
    widget.render(buf, state);
}

/// Renders the active picker overlay, dispatching on [`PickerKind`].
fn render_picker(frame: &mut Frame<'_>, area: Rect, state: &nullslop_domain::AppState) {
    match state.frontend.active_picker_kind {
        Some(PickerKind::Provider) => render_provider_picker(frame, area, state),
        Some(PickerKind::ContextAssembly) => {
            render_context_strategy_picker(frame, area, state);
        }
        Some(PickerKind::Keymap) => render_keymap_picker(frame, area, state),
        Some(PickerKind::Session) => render_session_picker(frame, area, state),
        None => {}
    }
}

/// Renders the provider picker overlay (delegates to slice).
fn render_provider_picker(frame: &mut Frame<'_>, area: Rect, state: &nullslop_domain::AppState) {
    nullslop_domain::feat::provider::render::render_provider_picker(frame, area, state);
}

/// Renders the context strategy picker overlay (delegates to slice).
fn render_context_strategy_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &nullslop_domain::AppState,
) {
    nullslop_domain::feat::picker::render::render_context_strategy_picker(frame, area, state);
}

/// Renders the keymap picker overlay (delegates to slice).
fn render_keymap_picker(frame: &mut Frame<'_>, area: Rect, state: &nullslop_domain::AppState) {
    nullslop_domain::feat::picker::render::render_keymap_picker(frame, area, state);
}

/// Renders the session picker overlay (delegates to slice).
fn render_session_picker(frame: &mut Frame<'_>, area: Rect, state: &nullslop_domain::AppState) {
    nullslop_domain::feat::session::render::render_session_picker(frame, area, state);
}

/// Renders a "terminal too small" message.
fn render_too_small(frame: &mut Frame<'_>, area: Rect, app: &mut TuiApp) {
    let msg = format!("Terminal too small\n{MIN_WIDTH}x{MIN_HEIGHT} minimum");
    let paragraph = Paragraph::new(msg).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
    // Clear selectable rects when terminal is too small.
    app.selectable_rects.rebuild(vec![]);
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod render_tests;
