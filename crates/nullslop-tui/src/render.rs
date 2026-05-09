//! Layout computation and rendering for the application.

use nullslop_protocol::{Mode, PickerKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui_tabs::{TabManager, TabsBar, TabsStyle};
use ratatui_which_key::{PopupPosition, WhichKey};

use crate::TuiApp;
use crate::app::{CHAT_PANE, PaneFocus};

/// Minimum terminal width.
pub const MIN_WIDTH: u16 = 40;
/// Minimum terminal height.
pub const MIN_HEIGHT: u16 = 14;

/// Maximum number of visible rows in the autocomplete popup.
const AUTOCOMPLETE_MAX_VISIBLE: usize = 20;
/// Minimum popup width.
const AUTOCOMPLETE_MIN_WIDTH: u16 = 20;
/// Maximum popup width as fraction of terminal width.
const AUTOCOMPLETE_MAX_WIDTH_FRAC: f32 = 0.60;
/// Separator between name and description.
const AUTOCOMPLETE_NAME_DESC_SEP: &str = " — ";
/// Text shown when no matches found.
const AUTOCOMPLETE_NO_MATCHES: &str = "<no prompts found>";

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

    match state.active_tab {
        nullslop_protocol::ActiveTab::Chat => {
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
                render_autocomplete_popup(frame, layout.input, &state);
            }
        }
        nullslop_protocol::ActiveTab::Dashboard => {
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

    if state.mode == Mode::Picker {
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
fn render_picker(frame: &mut Frame<'_>, area: Rect, state: &nullslop_component::AppState) {
    match state.active_picker_kind {
        Some(PickerKind::Provider) => render_provider_picker(frame, area, state),
        Some(PickerKind::ContextAssembly) => {
            render_context_strategy_picker(frame, area, state);
        }
        Some(PickerKind::Keymap) => render_keymap_picker(frame, area, state),
        Some(PickerKind::Session) => render_session_picker(frame, area, state),
        None => {}
    }
}

/// Renders the provider picker overlay using [`SelectionWidget`].
///
/// Telescope-style layout: bordered popup with filter input at top,
/// horizontal separator, scrollable results, and a footer line.
fn render_provider_picker(frame: &mut Frame<'_>, area: Rect, state: &nullslop_component::AppState) {
    use nullslop_component::provider_picker::entries;
    use nullslop_selection_widget::SelectionWidget;

    let footer = entries::format_footer(state.last_refreshed_at.as_ref(), area.width as usize);
    let widget = SelectionWidget::new(&state.provider_picker)
        .title(ratatui::text::Line::from(" Model "))
        .footer(footer);
    widget.render(frame, area);
}

/// Renders the context strategy picker overlay using [`SelectionWidget`].
///
/// Telescope-style layout: bordered popup with filter input at top,
/// horizontal separator, scrollable strategy entries, and a footer showing
/// the current strategy.
fn render_context_strategy_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &nullslop_component::AppState,
) {
    use nullslop_component::context_strategy_picker::entries;
    use nullslop_selection_widget::SelectionWidget;

    // Find the active strategy's display name for the footer.
    let active_name = state
        .context_strategy_picker
        .items()
        .iter()
        .find(|e| e.is_active)
        .map_or("unknown", |e| e.name.as_str());

    let footer = entries::format_strategy_footer(active_name);
    let widget = SelectionWidget::new(&state.context_strategy_picker)
        .title(ratatui::text::Line::from(" Context Assembly Strategy "))
        .footer(footer);
    widget.render(frame, area);
}

/// Renders the keymap picker overlay using [`SelectionWidget`].
///
/// Telescope-style layout: bordered popup with filter input at top,
/// horizontal separator, scrollable keymap entries, and a footer showing
/// the scope filter mode.
fn render_keymap_picker(frame: &mut Frame<'_>, area: Rect, state: &nullslop_component::AppState) {
    use nullslop_selection_widget::SelectionWidget;

    let scope_name = state
        .keymap_picker_origin_scope
        .as_deref()
        .unwrap_or("unknown");
    let footer = if state.keymap_picker_show_all {
        Line::from(format!(" All scopes | CTRL+A to show {scope_name} "))
    } else {
        Line::from(format!(" Scope: {scope_name} | CTRL+A to show all "))
    };
    let widget = SelectionWidget::new(&state.keymap_picker)
        .title(Line::from(" Keymaps "))
        .footer(footer);
    widget.render(frame, area);
}

/// Renders the session picker overlay using [`SelectionWidget`].
///
/// Telescope-style layout: bordered popup with filter input at top,
/// horizontal separator, scrollable session entries, and a footer showing
/// the CTRL+N shortcut.
fn render_session_picker(frame: &mut Frame<'_>, area: Rect, state: &nullslop_component::AppState) {
    use nullslop_selection_widget::SelectionWidget;

    let footer = Line::styled(
        "CTRL+N to create a new session",
        Style::default().fg(Color::Rgb(255, 165, 0)),
    );
    let widget = SelectionWidget::new(&state.session_picker)
        .title(Line::from(" Sessions "))
        .footer(footer);
    widget.render(frame, area);
}

/// Renders a "terminal too small" message.
fn render_too_small(frame: &mut Frame<'_>, area: Rect, app: &mut TuiApp) {
    let msg = format!("Terminal too small\n{MIN_WIDTH}x{MIN_HEIGHT} minimum");
    let paragraph = Paragraph::new(msg).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
    // Clear selectable rects when terminal is too small.
    app.selectable_rects.rebuild(vec![]);
}

/// Renders the autocomplete popup overlay above the input box.
///
/// The popup is a transient visual element — not a `UiElement`. It reads autocomplete
/// state directly from `AppState` and renders a bordered box with match entries.
/// The popup is horizontally anchored at the `$` token's screen column and sits
/// directly above the input box.
fn render_autocomplete_popup(
    frame: &mut Frame<'_>,
    input_area: Rect,
    state: &nullslop_component::AppState,
) {
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
fn scroll_window(selected: usize, total: usize, visible: usize) -> (usize, usize) {
    if total <= visible {
        return (0, total);
    }
    let start = (selected + 1).saturating_sub(visible);
    let end = (start + visible).min(total);
    (start, end)
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod render_tests;
