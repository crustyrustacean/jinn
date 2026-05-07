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
use crate::app::{CHAT_PANE, PaneFocus, WORKFLOW_PANE};

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
            let content_area = if app.workflow_pane_visible {
                app.split_manager.set_viewport(layout.content);
                let areas = app.split_manager.areas();
                let result = crate::split_borders::compute_split_borders(areas);
                let chat_rect = result.rect_for(CHAT_PANE).unwrap_or(layout.content);
                let workflow_rect = result
                    .rect_for(WORKFLOW_PANE)
.unwrap_or_default();

                // Render workflow panel into sidebar.
                if let Some(element) = app.ui_registry.get_mut("workflow-panel") {
                    element.render(frame, workflow_rect, &state);
                    if element.is_selectable() && app.pane_focus == PaneFocus::Workflow {
                        rects.push(workflow_rect);
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
        u16::try_from(visible_count + 2).unwrap_or(u16::MAX).min(input_area.y)
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
        for (i, m) in matches
            .iter()
            .enumerate()
            .skip(start)
            .take(end - start)
        {
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
mod tests {
    use super::*;
    use crate::selection::SelectionState;
    use nullslop_protocol::Command;
    use nullslop_selection_widget::compute_popup_rect;
    use ratatui::style::Modifier;

    #[test]
    fn app_layout_meets_min_size() {
        // Given a 40x14 area.
        let area = Rect::new(0, 0, 40, 14);

        // When checking meets_min_size.
        let result = AppLayout::meets_min_size(area);

        // Then it returns true.
        assert!(result);
    }

    #[test]
    fn app_layout_too_small() {
        // Given a 10x5 area.
        let area = Rect::new(0, 0, 10, 5);

        // When checking meets_min_size.
        let result = AppLayout::meets_min_size(area);

        // Then it returns false.
        assert!(!result);
    }

    #[test]
    fn init_tab_manager_has_two_tabs() {
        // Given a default tab manager.
        let mgr = init_tab_manager();

        // When checking tab count.
        // Then there are 2 tabs and the first is active.
        assert_eq!(mgr.tab_count(), 2);
        assert!(mgr.active_tab().is_some());
        assert_eq!(mgr.active_tab().unwrap().name, "Chat");
    }

    #[test]
    fn app_layout_includes_indicator_row() {
        // Given a 40x14 area.
        let area = Rect::new(0, 0, 40, 14);
        let layout = AppLayout::new(area, 1, 0);

        // Then the indicator row has height 1 and is between content and counter.
        assert_eq!(layout.indicator.height, 1);
        assert!(layout.indicator.y > layout.content.y);
        assert!(layout.indicator.y < layout.counter.y);
    }

    #[test]
    fn app_layout_queue_area_has_dynamic_height() {
        // Given a 40x20 area with 3 queued messages.
        let area = Rect::new(0, 0, 40, 20);
        let layout = AppLayout::new(area, 1, 3);

        // Then the queue area has height 3 and sits between indicator and counter.
        assert_eq!(layout.queue.height, 3);
        assert!(layout.queue.y > layout.indicator.y);
        assert!(layout.queue.y < layout.counter.y);
    }

    #[test]
    fn app_layout_queue_area_zero_height_when_empty() {
        // Given a 40x14 area with no queued messages.
        let area = Rect::new(0, 0, 40, 14);
        let layout = AppLayout::new(area, 1, 0);

        // Then the queue area has height 0.
        assert_eq!(layout.queue.height, 0);
    }

    #[test]
    fn app_layout_includes_status_bar() {
        // Given a 40x14 area.
        let area = Rect::new(0, 0, 40, 14);
        let layout = AppLayout::new(area, 1, 0);

        // Then the status bar has height 1 and is at the bottom.
        assert_eq!(layout.status_bar.height, 1);
        assert!(layout.status_bar.y > layout.input.y);
        assert_eq!(layout.status_bar.y + layout.status_bar.height, area.height);
    }

    // --- Provider picker rendering tests ---

    fn picker_state_with_ollama() -> (nullslop_component::AppState, nullslop_services::Services) {
        use nullslop_providers::{ProviderEntry, ProvidersConfig};
        let config = ProvidersConfig {
            providers: vec![ProviderEntry {
                name: "ollama".to_owned(),
                backend: "ollama".to_owned(),
                models: vec!["llama3".to_owned()],
                base_url: Some("http://localhost:11434".to_owned()),
                api_key_env: None,
                requires_key: false,
            }],
            aliases: vec![],
            default_provider: None,
        };
        let services = nullslop_services::test_services::TestServices::builder()
            .with_providers(config)
            .build();
        (nullslop_component::AppState::default(), services)
    }

    /// Helper to load provider entries into the picker state.
    fn load_picker_items(
        state: &mut nullslop_component::AppState,
        services: &nullslop_services::Services,
    ) {
        nullslop_component::provider_picker::load_provider_picker_items(services, state);
    }

    #[test]
    fn render_provider_picker_shows_telescope_layout() {
        // Given a terminal area and picker state with filter "ol".
        use nullslop_selection_widget::compute_popup_rect;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let (mut state, services) = picker_state_with_ollama();
        state.mode = Mode::Picker;
        state.active_picker_kind = Some(PickerKind::Provider);
        load_picker_items(&mut state, &services);
        state.provider_picker.insert_char('o');
        state.provider_picker.insert_char('l');

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        // When rendering the picker.
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_provider_picker(frame, area, &state);
            })
            .unwrap();

        // Then the popup contains the filter text with "> ol".
        let buffer = terminal.backend().buffer().clone();
        let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
        // Filter is on the first inner row: popup.y + 1
        let filter_y = popup.y + 1;
        let filter_cell = buffer.cell((popup.x + 1, filter_y)).expect("filter cell");
        assert_eq!(filter_cell.symbol(), ">");

        // Separator is on the second inner row.
        let sep_y = popup.y + 2;
        let sep_cell = buffer.cell((popup.x + 1, sep_y)).expect("sep cell");
        assert_eq!(sep_cell.symbol(), "\u{2500}");
    }

    #[test]
    fn render_provider_picker_height_scales_with_terminal() {
        // Given two terminal sizes.
        use nullslop_selection_widget::compute_popup_rect;

        let small_area = Rect::new(0, 0, 80, 24);
        let large_area = Rect::new(0, 0, 80, 42);

        // When computing popup rects.
        let small_popup = compute_popup_rect(small_area);
        let large_popup = compute_popup_rect(large_area);

        // Then the larger terminal gets a taller popup.
        assert!(large_popup.height > small_popup.height);

        // And the small terminal popup uses 75% of height + 4 rows of chrome.
        // floor(24 * 0.75) = 18, min(18 + 4, 24) = 22.
        assert_eq!(small_popup.height, 22);
    }

    #[test]
    fn render_provider_picker_uses_dark_gray_border() {
        // Given a picker render.
        use nullslop_selection_widget::compute_popup_rect;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let (mut state, services) = picker_state_with_ollama();
        load_picker_items(&mut state, &services);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                let area = frame.area();
                render_provider_picker(frame, area, &state);
            })
            .unwrap();

        // Then the border color is DarkGray, not Yellow.
        let buffer = terminal.backend().buffer().clone();
        let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
        let border_cell = buffer.cell((popup.x, popup.y)).expect("border cell");
        assert_eq!(border_cell.fg, Color::DarkGray);
    }

    #[test]
    fn render_provider_picker_shows_active_model_marker() {
        // Given a state with active_provider set to "ollama/llama3" and items loaded.
        use nullslop_selection_widget::compute_popup_rect;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let (mut state, services) = picker_state_with_ollama();
        state.mode = Mode::Picker;
        state.active_provider = "ollama/llama3".to_owned();
        state.active_picker_kind = Some(PickerKind::Provider);
        load_picker_items(&mut state, &services);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        // When rendering the picker.
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_provider_picker(frame, area, &state);
            })
            .unwrap();

        // Then the first result row starts with ">" (active marker) in green.
        let buffer = terminal.backend().buffer().clone();
        let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
        // Results start at popup.y + 3 (border + input + separator)
        let result_y = popup.y + 3;
        let marker_cell = buffer.cell((popup.x + 1, result_y)).expect("marker cell");
        assert_eq!(marker_cell.symbol(), ">");
        assert_eq!(marker_cell.fg, Color::Green);
    }

    // --- Context strategy picker rendering tests ---

    /// Helper to create a state with strategy entries loaded.
    fn strategy_picker_state() -> (nullslop_component::AppState, nullslop_services::Services) {
        let services = nullslop_services::Services::new();
        let mut state = nullslop_component::AppState::default();
        nullslop_component::context_strategy_picker::entries::load_strategy_picker_items(
            &services, &mut state,
        );
        (state, services)
    }

    #[test]
    fn render_context_strategy_picker_shows_telescope_layout() {
        // Given a terminal area and picker state with entries loaded.
        use nullslop_selection_widget::compute_popup_rect;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let (mut state, _services) = strategy_picker_state();
        state.mode = Mode::Picker;
        state.active_picker_kind = Some(PickerKind::ContextAssembly);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        // When rendering the picker.
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_context_strategy_picker(frame, area, &state);
            })
            .unwrap();

        // Then the popup shows telescope layout (filter marker and separator).
        let buffer = terminal.backend().buffer().clone();
        let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
        // Filter is on the first inner row: popup.y + 1
        let filter_y = popup.y + 1;
        let filter_cell = buffer.cell((popup.x + 1, filter_y)).expect("filter cell");
        assert_eq!(filter_cell.symbol(), ">");

        // Separator is on the second inner row.
        let sep_y = popup.y + 2;
        let sep_cell = buffer.cell((popup.x + 1, sep_y)).expect("sep cell");
        assert_eq!(sep_cell.symbol(), "\u{2500}");
    }

    #[test]
    fn render_context_strategy_picker_shows_active_marker() {
        // Given a state with entries (default is passthrough active).
        use nullslop_selection_widget::compute_popup_rect;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let (mut state, _services) = strategy_picker_state();
        state.mode = Mode::Picker;
        state.active_picker_kind = Some(PickerKind::ContextAssembly);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        // When rendering the picker.
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_context_strategy_picker(frame, area, &state);
            })
            .unwrap();

        // Then the first result row starts with ">" (active marker) in green.
        let buffer = terminal.backend().buffer().clone();
        let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
        // Results start at popup.y + 3 (border + input + separator)
        let result_y = popup.y + 3;
        let marker_cell = buffer.cell((popup.x + 1, result_y)).expect("marker cell");
        assert_eq!(marker_cell.symbol(), ">");
        assert_eq!(marker_cell.fg, Color::Green);
    }

    #[test]
    fn render_context_strategy_picker_shows_footer_with_current_strategy() {
        // Given a state with entries (default is passthrough active).
        use nullslop_selection_widget::compute_popup_rect;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let (mut state, _services) = strategy_picker_state();
        state.mode = Mode::Picker;
        state.active_picker_kind = Some(PickerKind::ContextAssembly);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        // When rendering the picker.
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_context_strategy_picker(frame, area, &state);
            })
            .unwrap();

        // Then the buffer contains "Current:" and "Passthrough" in the footer area.
        let buffer = terminal.backend().buffer().clone();
        let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
        // Footer is the last row of the inner area (before bottom border).
        let footer_y = popup.y + popup.height - 2;
        let row_text: String = (popup.x..popup.x + popup.width)
            .filter_map(|x| buffer.cell((x, footer_y)).map(ratatui::buffer::Cell::symbol))
            .collect();
        assert!(
            row_text.contains("Current:"),
            "footer should contain 'Current:' text, got: {row_text}"
        );
        assert!(
            row_text.contains("Passthrough"),
            "footer should contain 'Passthrough' text, got: {row_text}"
        );
    }

    // --- Keymap picker rendering tests ---

    fn keymap_picker_state() -> nullslop_component::AppState {
        use nullslop_component::keymap_picker::KeymapEntry;

        let mut state = nullslop_component::AppState::default();
        let entries = vec![
            KeymapEntry {
                key_sequence: "q".to_owned(),
                description: "quit".to_owned(),
                scope: "Normal".to_owned(),
                category: "General".to_owned(),
                command: Command::Quit,
                search_text: "q quit".to_owned(),
            },
            KeymapEntry {
                key_sequence: "gg".to_owned(),
                description: "scroll to top".to_owned(),
                scope: "Normal".to_owned(),
                category: "Navigation".to_owned(),
                command: Command::ScrollToTop,
                search_text: "gg scroll to top".to_owned(),
            },
            KeymapEntry {
                key_sequence: "<esc>".to_owned(),
                description: "set mode normal".to_owned(),
                scope: "Picker".to_owned(),
                category: "General".to_owned(),
                command: Command::SetMode {
                    payload: nullslop_protocol::system::SetMode {
                        mode: Mode::Normal,
                    },
                },
                search_text: "<esc> set mode normal".to_owned(),
            },
        ];
        state.keymap_picker.set_items(entries);
        state.mode = Mode::Picker;
        state.active_picker_kind = Some(PickerKind::Keymap);
        state.keymap_picker_origin_scope = Some("Normal".to_owned());
        state
    }

    #[test]
    fn render_keymap_picker_shows_telescope_layout() {
        // Given a terminal area with keymap picker state.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let state = keymap_picker_state();

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        // When rendering the picker.
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_keymap_picker(frame, area, &state);
            })
            .unwrap();

        // Then the buffer contains the title "Keymaps".
        let buffer = terminal.backend().buffer().clone();
        let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));

        // Title is on the first row (top border area).
        let title_row: String = (popup.x..popup.x + popup.width)
            .filter_map(|x| buffer.cell((x, popup.y)).map(ratatui::buffer::Cell::symbol))
            .collect();
        assert!(
            title_row.contains("Keymaps"),
            "title should contain 'Keymaps', got: {title_row}"
        );

        // Border color is DarkGray.
        let border_cell = buffer.cell((popup.x, popup.y)).expect("border cell");
        assert_eq!(border_cell.fg, Color::DarkGray);

        // Filter prompt is present on the first inner row.
        let filter_y = popup.y + 1;
        let filter_cell = buffer.cell((popup.x + 1, filter_y)).expect("filter cell");
        assert_eq!(filter_cell.symbol(), ">");
    }

    #[test]
    fn render_keymap_picker_footer_shows_current_scope() {
        // Given a keymap picker state with show_all = false and origin scope "Normal".
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut state = keymap_picker_state();
        state.keymap_picker_show_all = false;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        // When rendering the picker.
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_keymap_picker(frame, area, &state);
            })
            .unwrap();

        // Then the footer contains "Scope: Normal" and "CTRL+A".
        let buffer = terminal.backend().buffer().clone();
        let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
        let footer_y = popup.y + popup.height - 2;
        let footer_row: String = (popup.x..popup.x + popup.width)
            .filter_map(|x| buffer.cell((x, footer_y)).map(ratatui::buffer::Cell::symbol))
            .collect();
        assert!(
            footer_row.contains("Scope: Normal"),
            "footer should contain 'Scope: Normal', got: {footer_row}"
        );
        assert!(
            footer_row.contains("CTRL+A"),
            "footer should contain 'CTRL+A', got: {footer_row}"
        );
    }

    #[test]
    fn render_keymap_picker_footer_shows_all_scopes() {
        // Given a keymap picker state with show_all = true and origin scope "Normal".
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut state = keymap_picker_state();
        state.keymap_picker_show_all = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        // When rendering the picker.
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_keymap_picker(frame, area, &state);
            })
            .unwrap();

        // Then the footer contains "All scopes" and "CTRL+A to show Normal".
        let buffer = terminal.backend().buffer().clone();
        let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
        let footer_y = popup.y + popup.height - 2;
        let footer_row: String = (popup.x..popup.x + popup.width)
            .filter_map(|x| buffer.cell((x, footer_y)).map(ratatui::buffer::Cell::symbol))
            .collect();
        assert!(
            footer_row.contains("All scopes"),
            "footer should contain 'All scopes', got: {footer_row}"
        );
        assert!(
            footer_row.contains("CTRL+A to show Normal"),
            "footer should contain 'CTRL+A to show Normal', got: {footer_row}"
        );
    }

    // --- Selection highlight tests ---

    #[test]
    fn selection_highlight_inverts_cells_within_selection() {
        // Given a buffer with distinctively colored cells and an active selection.
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        // Paint a cell inside the selection with known colors.
        buf.cell_mut((3, 3)).unwrap().set_fg(Color::Yellow);
        buf.cell_mut((3, 3)).unwrap().set_bg(Color::Blue);
        // Paint a cell outside the selection with known colors.
        buf.cell_mut((15, 8)).unwrap().set_fg(Color::Red);
        buf.cell_mut((15, 8)).unwrap().set_bg(Color::Green);

        // And an app with an Active selection covering (2,2) to (5,4).
        let services = nullslop_services::Services::new();
        let mut app = crate::TuiApp::new(services);
        app.selection = SelectionState::Active {
            anchor: (2, 2),
            focus: (5, 4),
            bounds: area,
        };

        // When applying selection highlight.
        apply_selection_highlight(&app, &mut buf);

        // Then cell (3, 3) inside the selection has swapped fg/bg.
        let inside = buf.cell((3, 3)).expect("cell inside selection");
        assert_eq!(inside.fg, Color::Blue); // was bg
        assert_eq!(inside.bg, Color::Yellow); // was fg

        // And cell (15, 8) outside the selection is unchanged.
        let outside = buf.cell((15, 8)).expect("cell outside selection");
        assert_eq!(outside.fg, Color::Red);
        assert_eq!(outside.bg, Color::Green);
    }

    #[test]
    fn selection_highlight_respects_constraining_bounds() {
        // Given a buffer covering a large area and a selection where the raw anchor
        // extends beyond the selection's constraining bounds.
        let full_area = Rect::new(0, 0, 30, 30);
        let mut buf = ratatui::buffer::Buffer::empty(full_area);
        // Paint cell inside bounds (will be in clamped selection).
        buf.cell_mut((7, 7)).unwrap().set_fg(Color::Cyan);
        buf.cell_mut((7, 7)).unwrap().set_bg(Color::Magenta);
        // Paint cell at raw anchor position (0, 0) — outside bounds.
        buf.cell_mut((0, 0)).unwrap().set_fg(Color::White);
        buf.cell_mut((0, 0)).unwrap().set_bg(Color::Black);

        // And an Active selection with anchor outside bounds.
        // bounds=(5,5,10,10) means valid range is (5,5)-(14,14).
        // anchor=(0,0) is outside bounds, focus=(8,8) is inside.
        // selection_rect() should clamp to (5,5)-(8,8).
        let bounds = Rect::new(5, 5, 10, 10);
        let services = nullslop_services::Services::new();
        let mut app = crate::TuiApp::new(services);
        app.selection = SelectionState::Active {
            anchor: (0, 0),
            focus: (8, 8),
            bounds,
        };

        // When applying selection highlight.
        apply_selection_highlight(&app, &mut buf);

        // Then cell (7, 7) inside the clamped selection is inverted.
        let inside = buf.cell((7, 7)).expect("cell inside clamped selection");
        assert_eq!(inside.fg, Color::Magenta); // was bg
        assert_eq!(inside.bg, Color::Cyan); // was fg

        // And cell (0, 0) at the raw anchor position is NOT inverted.
        let outside = buf.cell((0, 0)).expect("cell at raw anchor");
        assert_eq!(outside.fg, Color::White); // unchanged
        assert_eq!(outside.bg, Color::Black); // unchanged
    }

    #[test]
    fn selection_highlight_does_nothing_when_idle() {
        // Given a buffer with distinctly colored cells and an Idle selection.
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        buf.cell_mut((5, 5)).unwrap().set_fg(Color::Yellow);
        buf.cell_mut((5, 5)).unwrap().set_bg(Color::Blue);

        // And an app with an Idle selection.
        let services = nullslop_services::Services::new();
        let mut app = crate::TuiApp::new(services);
        app.selection = SelectionState::Idle;

        // When applying selection highlight.
        apply_selection_highlight(&app, &mut buf);

        // Then no cells are inverted — colors remain unchanged.
        let cell = buf.cell((5, 5)).expect("colored cell");
        assert_eq!(cell.fg, Color::Yellow); // unchanged
        assert_eq!(cell.bg, Color::Blue); // unchanged
    }

    #[test]
    fn selection_highlight_uses_explicit_colors_when_fg_equals_bg() {
        // Given a buffer where cells have matching fg and bg (e.g. both Reset,
        // as with user messages rendered with Style::default().bold()).
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        // User-message-style cell: fg = Reset, bg = Reset (bold modifier).
        buf.cell_mut((3, 3))
            .unwrap()
            .set_style(Style::default().add_modifier(Modifier::BOLD));
        // Adjacent cell with distinct colors (assistant-style).
        buf.cell_mut((4, 3)).unwrap().set_fg(Color::Cyan);

        // And an Active selection covering both cells.
        let services = nullslop_services::Services::new();
        let mut app = crate::TuiApp::new(services);
        app.selection = SelectionState::Active {
            anchor: (2, 2),
            focus: (5, 4),
            bounds: area,
        };

        // When applying selection highlight.
        apply_selection_highlight(&app, &mut buf);

        // Then the Reset/Reset cell gets explicit highlight colors.
        let reset_cell = buf.cell((3, 3)).expect("reset cell");
        assert_eq!(reset_cell.fg, Color::Black);
        assert_eq!(reset_cell.bg, Color::White);

        // And the distinct-colors cell gets swapped fg/bg.
        let cyan_cell = buf.cell((4, 3)).expect("cyan cell");
        assert_eq!(cyan_cell.fg, Color::Reset); // was bg
        assert_eq!(cyan_cell.bg, Color::Cyan); // was fg
    }

    // --- Clipboard flush tests ---

    #[test]
    fn clipboard_copy_clears_pending_flag_on_idle_selection() {
        // Given an app with pending_clipboard set but Idle selection.
        let services = nullslop_services::Services::new();
        let mut app = crate::TuiApp::new(services);
        app.selection = SelectionState::Idle;
        app.pending_clipboard = true;

        let area = Rect::new(0, 0, 20, 5);
        let buf = ratatui::buffer::Buffer::empty(area);

        // When flushing the pending clipboard.
        flush_pending_clipboard(&mut app, &buf);

        // Then the pending flag is cleared (even though there was nothing to copy).
        assert!(!app.pending_clipboard);
    }

    #[test]
    fn clipboard_copy_skips_empty_selection() {
        // Given an app with pending_clipboard and an Active selection over empty cells.
        let area = Rect::new(0, 0, 20, 5);
        let buf = ratatui::buffer::Buffer::empty(area);

        let services = nullslop_services::Services::new();
        let mut app = crate::TuiApp::new(services);
        app.selection = SelectionState::Active {
            anchor: (0, 0),
            focus: (3, 0),
            bounds: area,
        };
        app.pending_clipboard = true;

        // When flushing the pending clipboard.
        flush_pending_clipboard(&mut app, &buf);

        // Then the pending flag is cleared.
        assert!(!app.pending_clipboard);
    }

    #[test]
    #[ignore = "requires clipboard access (run with --ignored)"]
    fn clipboard_copy_extracts_selected_text() {
        // Given a buffer with known text and an active selection.
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        // Write "Hello" on row 2.
        for (i, ch) in "Hello".chars().enumerate() {
            buf.cell_mut((2 + i as u16, 2))
                .unwrap()
                .set_symbol(&ch.to_string());
        }

        let services = nullslop_services::Services::new();
        let mut app = crate::TuiApp::new(services);
        app.selection = SelectionState::Active {
            anchor: (2, 2),
            focus: (6, 2),
            bounds: area,
        };
        app.pending_clipboard = true;

        // When flushing the pending clipboard.
        flush_pending_clipboard(&mut app, &buf);

        // Then the pending flag is cleared immediately.
        assert!(!app.pending_clipboard);

        // And after the clipboard thread completes, the clipboard contains
        // the selected text.
        std::thread::sleep(std::time::Duration::from_millis(500));
        let mut clipboard = arboard::Clipboard::new().expect("clipboard access");
        let content = clipboard.get_text().expect("read clipboard");
        assert_eq!(content, "Hello");
    }

    // --- Element-driven selectable rect tests ---

    #[test]
    fn render_registers_content_rect_for_selectable_chat_log() {
        // Given a TuiApp rendered in Chat tab with a 80x24 terminal.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let services = nullslop_services::Services::new();
        let mut app = crate::TuiApp::new(services);
        // Default tab is Chat.

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        // When rendering.
        terminal
            .draw(|frame| {
                app.render(frame);
            })
            .unwrap();

        // Then the content area rect is registered as selectable.
        // Chat log is selectable, so layout.content should be in selectable_rects.
        let layout = AppLayout::new(frame_area(80, 24), 1, 0);
        let found = app
            .selectable_rects
            .find_for_position(layout.content.x + 1, layout.content.y + 1);
        assert!(
            found.is_some(),
            "chat log content rect should be selectable"
        );
        assert_eq!(found.unwrap(), layout.content);
    }

    #[test]
    fn render_registers_picker_popup_rect_when_active() {
        // Given a TuiApp rendered with Mode::Picker.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let services = nullslop_services::Services::new();
        let mut app = crate::TuiApp::new(services);
        // Switch to Picker mode with an active provider picker.
        app.core.state.write().mode = nullslop_protocol::Mode::Picker;
        app.core.state.write().active_picker_kind = Some(nullslop_protocol::PickerKind::Provider);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        // When rendering.
        terminal
            .draw(|frame| {
                app.render(frame);
            })
            .unwrap();

        // Then the picker popup rect is registered as selectable.
        let popup_rect = compute_popup_rect(Rect::new(0, 0, 80, 24));
        // Query position (popup.x + 1, 0) — inside popup, but above the content area (y=1)
        // so the smallest matching rect is the picker popup, not the content.
        let found = app
            .selectable_rects
            .find_for_position(popup_rect.x + 1, 0);
        assert!(found.is_some(), "picker popup rect should be selectable");
        assert_eq!(found.unwrap(), popup_rect);

        // And the content area rect is also still selectable (chat-log is selectable).
        let layout = AppLayout::new(frame_area(80, 24), 1, 0);
        let content_found = app
            .selectable_rects
            .find_for_position(layout.content.x + 1, layout.content.y + 1);
        assert!(
            content_found.is_some(),
            "content rect should also be selectable alongside picker"
        );
    }

    // --- Autocomplete popup rendering tests ---

    /// Helper to create an `AppState` with autocomplete active.
    ///
    /// Sets the input buffer to the given text, activates autocomplete at `token_start`,
    /// and populates matches.
    fn state_with_autocomplete(
        buffer_text: &str,
        token_start: usize,
        matches: Vec<nullslop_component::chat_input_box::state::AutocompleteMatch>,
    ) -> nullslop_component::AppState {
        let mut state = nullslop_component::AppState::default();
        state.active_chat_input_mut().replace_all(buffer_text.to_owned());
        // Position cursor after the buffer text.
        // Note: cursor must be at the end for autocomplete to be consistent.
        state
            .active_chat_input_mut()
            .activate_autocomplete(token_start, matches);
        state
    }

    /// Extract a line of text from a buffer at the given row.
    fn buffer_line(buf: &ratatui::buffer::Buffer, y: u16, start_x: u16, max_len: u16) -> String {
        let mut s = String::new();
        for x in start_x..start_x + max_len {
            let cell = buf.cell((x, y));
            let sym = cell.map_or(" ", ratatui::buffer::Cell::symbol);
            if sym == " " && s.ends_with("  ") {
                break;
            }
            s.push_str(sym);
        }
        s.trim_end().to_owned()
    }

    #[test]
    fn render_autocomplete_popup_shows_matches() {
        // Given an AppState with autocomplete active and 3 matches.
        use nullslop_component::chat_input_box::state::AutocompleteMatch;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let matches = vec![
            AutocompleteMatch {
                name: "code-review".to_owned(),
                description: "Perform code review".to_owned(),
            },
            AutocompleteMatch {
                name: "summarize".to_owned(),
                description: "Summarize text".to_owned(),
            },
            AutocompleteMatch {
                name: "test-gen".to_owned(),
                description: "Generate tests".to_owned(),
            },
        ];
        let state = state_with_autocomplete("$co", 0, matches);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        // When rendering the autocomplete popup with a known input area.
        let input_area = Rect::new(0, 20, 80, 4);
        terminal
            .draw(|frame| {
                render_autocomplete_popup(frame, input_area, &state);
            })
            .unwrap();

        // Then the popup shows all three matches.
        let buffer = terminal.backend().buffer().clone();
        let popup_top = 20 - 5; // 3 matches + 2 border rows = 5, popup sits above input_area
        // Check that match names appear in the popup content.
        let line1 = buffer_line(&buffer, popup_top + 1, 1, 60);
        let line2 = buffer_line(&buffer, popup_top + 2, 1, 60);
        let line3 = buffer_line(&buffer, popup_top + 3, 1, 60);
        assert!(line1.contains("code-review"), "first match should contain 'code-review', got: {line1}");
        assert!(line2.contains("summarize"), "second match should contain 'summarize', got: {line2}");
        assert!(line3.contains("test-gen"), "third match should contain 'test-gen', got: {line3}");
    }

    #[test]
    fn render_autocomplete_popup_highlights_selected() {
        // Given an AppState with 2 matches and the second (most-relevant) selected.
        use nullslop_component::chat_input_box::state::AutocompleteMatch;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let matches = vec![
            AutocompleteMatch {
                name: "alpha".to_owned(),
                description: String::new(),
            },
            AutocompleteMatch {
                name: "beta".to_owned(),
                description: String::new(),
            },
        ];
        let mut state = state_with_autocomplete("$", 0, matches);
        // Default selected_index is last (index 1 = "beta").
        // Move selection up to select index 0 ("alpha").
        state.active_chat_input_mut().autocomplete_move_up();

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let input_area = Rect::new(0, 20, 80, 4);

        terminal
            .draw(|frame| {
                render_autocomplete_popup(frame, input_area, &state);
            })
            .unwrap();

        // Then the selected row has Modifier::REVERSED.
        // Popup: anchor_x = 0 + 2 (prompt_indent) + 0 (token_col) = 2.
        // Popup at (2, 16, 20, 4). Content starts at x=3.
        // First match (index 0, selected) at y=17, second at y=18.
        let buffer = terminal.backend().buffer().clone();
        let selected_cell = buffer.cell((3, 17)).expect("selected cell");
        assert!(
            selected_cell.modifier.contains(Modifier::REVERSED),
            "selected cell should have REVERSED modifier"
        );

        // Second match (index 1) is NOT selected.
        let unselected_cell = buffer.cell((3, 18)).expect("unselected cell");
        assert!(
            !unselected_cell.modifier.contains(Modifier::REVERSED),
            "unselected cell should NOT have REVERSED modifier"
        );
    }

    #[test]
    fn render_autocomplete_popup_shows_no_matches_message() {
        // Given an AppState with autocomplete active but 0 matches.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let state = state_with_autocomplete("$xyz", 0, vec![]);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let input_area = Rect::new(0, 20, 80, 4);

        terminal
            .draw(|frame| {
                render_autocomplete_popup(frame, input_area, &state);
            })
            .unwrap();

        // Then the popup shows "<no prompts found>".
        let buffer = terminal.backend().buffer().clone();
        let popup_top = 20 - 3; // 1 content + 2 borders
        let line = buffer_line(&buffer, popup_top + 1, 1, 60);
        assert!(
            line.contains("<no prompts found>"),
            "should show no matches message, got: {line}"
        );
    }

    #[test]
    fn render_autocomplete_popup_positioned_above_input() {
        // Given a known input area at row 20.
        use nullslop_component::chat_input_box::state::AutocompleteMatch;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let matches = vec![
            AutocompleteMatch {
                name: "test".to_owned(),
                description: "A test".to_owned(),
            },
        ];
        let state = state_with_autocomplete("$", 0, matches);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let input_area = Rect::new(0, 20, 80, 4);

        terminal
            .draw(|frame| {
                render_autocomplete_popup(frame, input_area, &state);
            })
            .unwrap();

        // Then the popup's bottom edge touches input_area.y.
        // Popup: anchor_x = 0 + 2 + 0 = 2. Height = 1 + 2 = 3.
        // popup_y = 20 - 3 = 17. Bottom border at y = 19.
        let buffer = terminal.backend().buffer().clone();
        let border_cell = buffer.cell((2, 19)).expect("bottom border cell");
        assert_eq!(
            border_cell.fg,
            Color::DarkGray,
            "bottom border of popup should be at row 19, x=2 (popup anchor)"
        );
    }

    #[test]
    fn render_autocomplete_popup_anchored_at_dollar() {
        // Given a buffer "foo $co" — the $ is at grapheme index 4.
        use nullslop_component::chat_input_box::state::AutocompleteMatch;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let matches = vec![
            AutocompleteMatch {
                name: "code".to_owned(),
                description: "Code stuff".to_owned(),
            },
        ];
        let state = state_with_autocomplete("foo $co", 4, matches);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        // Input area starts at x=10 to see horizontal anchoring.
        let input_area = Rect::new(10, 20, 70, 4);

        terminal
            .draw(|frame| {
                render_autocomplete_popup(frame, input_area, &state);
            })
            .unwrap();

        // Then the popup's left edge is near the $ column.
        // $ is at grapheme index 4 in the buffer, col 4 on the first line.
        // Input inner starts at x=10, prompt_indent=2, so anchor_x = 10 + 2 + 4 = 16.
        let buffer = terminal.backend().buffer().clone();
        let popup_top = 20 - 3; // 1 match + 2 borders
        // Top-left corner of the popup should be at or near x=16.
        let corner_cell = buffer.cell((16, popup_top)).expect("popup corner");
        assert_eq!(
            corner_cell.fg,
            Color::DarkGray,
            "popup left border should be anchored at $ column (x=16)"
        );
    }

    #[test]
    fn render_autocomplete_popup_width_based_on_content() {
        // Given matches with varying name lengths.
        use nullslop_component::chat_input_box::state::AutocompleteMatch;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let matches = vec![
            AutocompleteMatch {
                name: "short".to_owned(),
                description: "s".to_owned(),
            },
            AutocompleteMatch {
                name: "a-very-long-template-name".to_owned(),
                description: "A very long description indeed".to_owned(),
            },
        ];
        let state = state_with_autocomplete("$", 0, matches);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let input_area = Rect::new(0, 20, 80, 4);

        terminal
            .draw(|frame| {
                render_autocomplete_popup(frame, input_area, &state);
            })
            .unwrap();

        // Then the popup width accommodates the longest line plus borders.
        let buffer = terminal.backend().buffer().clone();
        let popup_top = 20 - 4; // 2 matches + 2 borders
        // The longest line: "a-very-long-template-name — A very long description indeed"
        // Check that the longer match text is visible in the buffer.
        let long_line = buffer_line(&buffer, popup_top + 2, 1, 60);
        assert!(
            long_line.contains("a-very-long-template-name"),
            "long name should be visible, got: {long_line}"
        );
    }

    #[test]
    fn render_autocomplete_popup_does_not_render_when_inactive() {
        // Given an AppState with autocomplete inactive.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let state = nullslop_component::AppState::default();

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let input_area = Rect::new(0, 20, 80, 4);

        terminal
            .draw(|frame| {
                render_autocomplete_popup(frame, input_area, &state);
            })
            .unwrap();

        // Then no popup renders — the buffer should remain empty (default space chars).
        let buffer = terminal.backend().buffer().clone();
        // Check an area above the input where the popup would be.
        let cell = buffer.cell((0, 15)).expect("cell should exist");
        assert_eq!(
            cell.symbol(),
            " ",
            "no popup content should appear when autocomplete is inactive"
        );
    }

    /// Helper to create a Rect matching the terminal dimensions.
    fn frame_area(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }
}
