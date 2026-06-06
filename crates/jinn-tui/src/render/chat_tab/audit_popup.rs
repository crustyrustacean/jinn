//! Audit popup overlay - renders the context-change history of the
//! currently-selected chat entry.
//!
//! Activated by pressing `a` in Normal mode (toggles
//! `FrontendState::audit_popup_visible`). The popup is right-aligned to the
//! chat-log area and vertically anchored to the top of the selected entry.
//! It tracks the cursor live as the user navigates.

use jinn_domain::RenderCtx;
use jinn_domain::common::focus::FocusScope;
use jinn_domain::feat::ui::chat_log::audit_popup::{audit_popup_rect, format_audit_lines};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// Render the audit popup, if it should be visible.
///
/// Conditions for visibility:
/// - `audit_popup_visible` is true
/// - No higher-priority overlay is active
/// - An entry is selected
///
/// `chat_log_area` is the chat-log content area (right-aligned anchor and
// bottom-edge clamp).
pub(super) fn render_audit_popup(
    frame: &mut Frame<'_>,
    chat_log_area: Rect,
    ctx: &RenderCtx,
    rects: &mut Vec<Rect>,
) {
    if !ctx.state.frontend.audit_popup_visible {
        return;
    }

    // Suppress when a higher-priority overlay is active.
    if overlay_active(ctx) {
        return;
    }

    let Some(entry) = ctx.state.active_session().selected_entry() else {
        return;
    };

    let lines = format_audit_lines(entry, &ctx.state.frontend.theme);

    // Resolve the screen Y of the selected entry's top edge.
    // Returns None until the render pipeline has populated cached fields for
    // this frame; in that case we simply skip rendering for this frame.
    let Some(entry_top_y) = ctx
        .state
        .active_session()
        .selected_entry_screen_y(chat_log_area.y)
    else {
        return;
    };

    let rect = audit_popup_rect(chat_log_area, entry_top_y, lines.len());

    // Clear underlying buffer so the popup is opaque.
    frame.render_widget(Clear, rect);

    // Render the popup body. The block provides top+bottom borders; the
    // header line is the first body line (so it scrolls with content).
    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .style(
            Style::default()
                .bg(ctx.state.frontend.theme.infopopup_bg)
                .fg(ctx.state.frontend.theme.infopopup_border),
        );
    let paragraph = Paragraph::new(lines).style(
        Style::default()
            .bg(ctx.state.frontend.theme.infopopup_bg)
            .fg(ctx.state.frontend.theme.infopopup_fg),
    );
    frame.render_widget(paragraph.block(block), rect);

    rects.push(rect);
}

/// Returns true if a higher-priority overlay is currently active.
fn overlay_active(ctx: &RenderCtx) -> bool {
    matches!(
        ctx.state.frontend.scope_stack.current(),
        FocusScope::Picker { .. }
            | FocusScope::ArgInput
            | FocusScope::RenameSessionInput
            | FocusScope::SidebarSessions
    )
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "test code, panics are acceptable"
    )]
    //! Render-level tests for the audit popup overlay.
    //!
    //! These tests exercise the integration of:
    //! - `format_audit_lines` (domain pure function)
    //! - `selected_entry_screen_y` (session helper)
    //! - `audit_popup_rect` (rect computation)
    //! - The widget stack (Clear + Block + Paragraph)
    //! - The `rects.push()` registration for mouse-selectable regions.
    //!
    //! Together they pin the contract that the popup paints at the computed
    //! rect with the expected text and is registered as a selectable region.
    use jinn_domain::RenderCtx;
    use jinn_domain::FocusScope;
    use jinn_domain::feat::session::chat_entry::{ChangeSource, ChatEntry, ContextOverride};
    use jinn_testutil::setup_term;
    use ratatui::layout::Rect;

    use super::render_audit_popup;

    /// Width of the audit popup (matches `AUDIT_POPUP_WIDTH` in the domain).
    /// Duplicated here because the constant is private to the domain crate.
    const AUDIT_POPUP_WIDTH: u16 = 60;

    /// Build an app with one user entry that has one audit event, with the
    /// audit popup toggle ON.
    fn app_with_audit_visible() -> crate::TuiApp {
        let app = crate::TuiApp::test_builder().build();
        let mut entry = ChatEntry::user("hello");
        entry.apply_context_override(
            ContextOverride::ForcedExclude,
            ChangeSource::User,
        );
        app.core.state.write().frontend.audit_popup_visible = true;
        app.core.state.write().active_session_mut().push_entry(entry);
        app
    }

    /// Returns the audit popup rect by probing `selectable_rects` for a
    /// rect of width `AUDIT_POPUP_WIDTH`. Returns `None` if not found.
    ///
    /// Probes the rightmost column of the chat-log area across all visible
    /// rows; the audit popup is right-aligned to the chat-log area so its
    /// right edge sits at `chat_log_area.x + chat_log_area.width - 1`. The
    /// first probe hit yields the popup rect.
    fn find_audit_popup_rect(app: &crate::TuiApp) -> Option<ratatui::layout::Rect> {
        // Scan every cell of a 80x24 frame and return the first rect of width
        // AUDIT_POPUP_WIDTH that we find. This is slower than a single probe but
        // robust to layout changes (e.g. sidebar width, content area offsets).
        for y in 0..24 {
            for x in 0..80 {
                if let Some(rect) = app.selectable_rects.find_for_position(x, y) {
                    if rect.width == AUDIT_POPUP_WIDTH {
                        return Some(rect);
                    }
                }
            }
        }
        None
    }

    #[rstest::rstest]
    fn render_audit_popup_paints_header_and_body_at_computed_rect() {
        // Given a state with audit visible and one excluded entry, with
        // pre-populated line ranges (simulating what render_chat_log does).
        let app = app_with_audit_visible();
        let (mut terminal, _area) = setup_term(80, 24);

        // Pre-populate the line-range cache that render_chat_log normally fills.
        // The selected entry occupies wrapped-line 0..=0 (one line).
        {
            let mut wstate = app.core.state.write();
            let session = wstate.active_session_mut();
            session.set_entry_line_ranges(vec![(0, 0)]);
            session.set_rendered_scroll_offset(0);
            session.set_viewport_height(24);
            session.set_blank_count(0);
        }

        // The chat-log area we render against. Wide enough to fit the
        // 60-col popup with room to spare.
        let chat_log_area = Rect::new(30, 0, 70, 24);

        // When rendering the popup directly.
        let mut rects: Vec<Rect> = Vec::new();
        terminal
            .draw(|frame| {
                let guard = app.core.state.read();
                let ctx = RenderCtx::new(&guard);
                render_audit_popup(frame, chat_log_area, &ctx, &mut rects);
            })
            .unwrap();

        // Then exactly one rect is registered, matching the popup width.
        assert_eq!(rects.len(), 1, "exactly one popup rect should be registered");
        let popup = rects[0];
        assert_eq!(popup.width, AUDIT_POPUP_WIDTH, "popup width");
        assert_eq!(
            popup.x + popup.width,
            chat_log_area.x + chat_log_area.width,
            "popup right edge should align to chat-log right edge"
        );

        // And the popup height accommodates header + body + 2 borders
        // (1 header + 1 body = 2 content lines + 2 borders = 4).
        assert_eq!(popup.height, 4, "popup height should be content + 2 borders");

        // And the rendered buffer contains the header text on the first body
        // line (popup.y + 1, since row 0 is the top border).
        // And the rendered buffer contains the header text on the first body
        // line (popup.y + 1, since row 0 is the top border).
        let buffer = terminal.backend().buffer();
        let header_y = popup.y + 1;
        let header_row: String = (popup.x..popup.x + popup.width)
            .filter_map(|x| buffer.cell((x, header_y)).map(|c| c.symbol().to_owned()))
            .collect();
        assert!(
            header_row.contains("audit")
                && header_row.contains("1 events")
                && header_row.contains("ForcedExclude"),
            "header row at y={header_y}: {header_row:?}"
        );

        // And the rendered buffer contains the body line text on the second
        // body line (popup.y + 2).
        let body_y = popup.y + 2;
        let body_row: String = (popup.x..popup.x + popup.width)
            .filter_map(|x| buffer.cell((x, body_y)).map(|c| c.symbol().to_owned()))
            .collect();
        assert!(
            body_row.contains("user") && body_row.contains("Default"),
            "body row at y={body_y}: {body_row:?}"
        );
    }

    #[rstest::rstest]
    fn render_audit_popup_skips_when_visibility_off() {
        // Given a state with audit popup visibility OFF.
        let mut app = crate::TuiApp::test_builder().build();
        let mut entry = ChatEntry::user("hello");
        entry
            .apply_context_override(ContextOverride::ForcedExclude, ChangeSource::User);
        // audit_popup_visible stays at default (false)
        app.core.state.write().active_session_mut().push_entry(entry);
        let (mut terminal, _area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                app.render(frame);
            })
            .unwrap();

        // Then no audit popup rect is registered (findable via probing).
        assert!(
            find_audit_popup_rect(&app).is_none(),
            "no audit popup rect should be registered when visibility is off"
        );

        // And the rendered buffer does not contain audit text anywhere.
        let buffer = terminal.backend().buffer();
        for y in 0..24 {
            for x in 0..80 {
                if let Some(cell) = buffer.cell((x, y)) {
                    let s = cell.symbol();
                    if s.contains("audit") && s.contains("events") {
                        panic!(
                            "audit text should not appear when popup is off (found at ({x}, {y}))"
                        );
                    }
                }
            }
        }
    }

    #[rstest::rstest]
    fn render_audit_popup_skips_when_picker_overlay_active() {
        // Given a state with audit visible BUT a Picker overlay on top.
        let mut app = app_with_audit_visible();
        app.core
            .state
            .write()
            .frontend
            .scope_stack
            .push(FocusScope::Picker {
                kind: jinn_domain::PickerKind::Provider,
            });
        let (mut terminal, _area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                app.render(frame);
            })
            .unwrap();

        // Then no audit popup rect was registered (suppressed by overlay).
        assert!(
            find_audit_popup_rect(&app).is_none(),
            "audit popup should be suppressed when Picker overlay is active"
        );
    }
}
