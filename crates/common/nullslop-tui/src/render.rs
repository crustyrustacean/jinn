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
    #[must_use]
    pub fn new(area: Rect, input_lines: u16, queue_lines: u16) -> Self {
        let [tabs, rest] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);

        let input_height = 2 + input_lines.max(1); // top border + text + bottom border
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

    let state = app.core.state.read();
    let queue_len = state.active_session().queue_len() as u16;
    let layout = AppLayout::new(
        area,
        state.active_chat_input().visual_line_count() as u16,
        queue_len,
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
                nullslop_domain::chat_input_box::autocomplete_render::render_autocomplete_popup(
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

/// Inverts foreground and background for cells within the active selection rect.
///
/// This is a post-rendering pass applied after all UI elements have drawn.
/// The selection rect comes from [`SelectionState::selection_rect()`], which is
/// already normalized and clamped to the constraining bounds.
fn apply_selection_highlight(app: &TuiApp, buf: &mut ratatui::buffer::Buffer) {
    if let Some(sel_rect) = app.selection.selection_rect() {
        for y in sel_rect.top()..sel_rect.bottom() {
            for x in sel_rect.left()..sel_rect.right() {
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
    nullslop_domain::provider::render::render_provider_picker(frame, area, state);
}

/// Renders the context strategy picker overlay (delegates to slice).
fn render_context_strategy_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &nullslop_domain::AppState,
) {
    nullslop_domain::picker::render::render_context_strategy_picker(frame, area, state);
}

/// Renders the keymap picker overlay (delegates to slice).
fn render_keymap_picker(frame: &mut Frame<'_>, area: Rect, state: &nullslop_domain::AppState) {
    nullslop_domain::picker::render::render_keymap_picker(frame, area, state);
}

/// Renders the session picker overlay (delegates to slice).
fn render_session_picker(frame: &mut Frame<'_>, area: Rect, state: &nullslop_domain::AppState) {
    nullslop_domain::session::render::render_session_picker(frame, area, state);
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
