//! Render-level tests for the chat log's audit popup overlay.
//!
//! These tests need a `TuiApp`, so they stay with the composition
//! layer (Rule E) and call the slice's draw function directly rather
//! than reaching it through a registered slot. They pin the contract
//! that the popup paints at the computed rect with the expected text
//! and registers exactly one mouse-selectable region.
#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::manual_assert,
        clippy::panic,
        clippy::map_unwrap_or,
        clippy::redundant_closure_for_method_calls,
        clippy::collapsible_if,
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
    use jinn_chat_log_view::chat_log::AUDIT_POPUP_WIDTH;
    use jinn_kernel::RenderCtx;
    use jinn_kernel::protocol::{ChangeSource, ChatEntry, ContextOverride};
    use jinn_slices::FocusScope;
    use jinn_testutil::setup_term;
    use ratatui::layout::Rect;

    use jinn_chat_log_view::render_regions::render_audit_popup;

    /// Build an app with one user entry that has one audit event, with the
    /// audit popup toggle ON.
    async fn app_with_audit_visible() -> crate::TuiApp {
        let app = crate::TuiApp::test_builder().build().await;
        let mut entry = ChatEntry::user("hello");
        entry.apply_context_override(ContextOverride::ForcedExclude, ChangeSource::User);
        let _ = jinn_chat_log_view::audit_popup::toggle(&app.services.slices);
        app.core
            .state
            .write()
            .active_session_mut()
            .push_entry(entry);
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

    /// Render the audit popup into a 100×24 terminal and return a snapshot
    /// of the rendered buffer, the chat-log area it was rendered against, and
    /// every rect the popup registered.
    ///
    /// Mirrors the paint test's setup: one excluded entry, audit visible,
    /// pre-populated line ranges, popup rendered right-aligned in a 70-col
    /// chat-log area.
    async fn render_popup_buffer() -> (ratatui::buffer::Buffer, Rect, Vec<Rect>) {
        let app = app_with_audit_visible().await;
        let (mut terminal, _area) = setup_term(100, 24);

        {
            let mut wstate = app.core.state.write();
            let session = wstate.active_session_mut();
            session.set_entry_line_ranges(vec![(0, 0)]);
            session.set_rendered_scroll_offset(0);
            session.set_viewport_height(24);
            session.set_blank_count(0);
        }

        let chat_log_area = Rect::new(30, 0, 70, 24);

        let mut rects: Vec<Rect> = Vec::new();
        terminal
            .draw(|frame| {
                let guard = app.core.state.read();
                // The app's own registry, not a throwaway one: the popup's
                // visibility is a chat-log cell, and an empty registry
                // would read as hidden and make these tests vacuous.
                let views = jinn_slices::OverlayViews::new();
                let ctx = RenderCtx::new_with_default_config(&guard, &app.services.slices, &views);
                render_audit_popup(frame, chat_log_area, &ctx, &mut rects);
            })
            .unwrap();

        (terminal.backend().buffer().clone(), chat_log_area, rects)
    }

    /// The single rect the popup registered, or a panic naming the count.
    fn only_popup_rect(rects: &[Rect]) -> Rect {
        assert_eq!(
            rects.len(),
            1,
            "exactly one popup rect should be registered"
        );
        rects[0]
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn render_audit_popup_registers_one_rect_at_the_computed_geometry() {
        // Given audit visible with one excluded entry and pre-populated line
        // ranges, rendered into a chat-log area wide enough for the popup.
        let (_buffer, chat_log_area, rects) = render_popup_buffer().await;

        // When reading the rect the popup registered.
        let popup = only_popup_rect(&rects);

        // Then its width matches the popup width.
        assert_eq!(popup.width, AUDIT_POPUP_WIDTH, "popup width");
        // And its right edge aligns to the chat-log right edge.
        assert_eq!(
            popup.x + popup.width,
            chat_log_area.x + chat_log_area.width,
            "popup right edge should align to chat-log right edge"
        );
        // And its height is content + 2 borders: 3 Metadata lines
        // (title, Sent, blank) + 1 audit header + 1 audit body = 7.
        assert_eq!(
            popup.height, 7,
            "popup height should be content + 2 borders"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn render_audit_popup_paints_the_metadata_block_body_rows() {
        // Given the audit popup rendered with one excluded entry.
        let (buffer, _chat_log_area, rects) = render_popup_buffer().await;
        let popup = only_popup_rect(&rects);

        // When reading the first two body rows (row 0 is the top border).
        let metadata_y = popup.y + 1;
        let sent_y = popup.y + 2;
        let row_text = |y: u16| -> String {
            (popup.x..popup.x + popup.width)
                .filter_map(|x| buffer.cell((x, y)).map(|c| c.symbol().to_owned()))
                .collect()
        };
        let metadata_row = row_text(metadata_y);
        let sent_row = row_text(sent_y);

        // Then the first carries the Metadata title.
        assert!(
            metadata_row.contains("Metadata"),
            "metadata title row at y={metadata_y}: {metadata_row:?}"
        );
        // And the second carries the Sent line.
        assert!(
            sent_row.contains("Sent:"),
            "sent row at y={sent_y}: {sent_row:?}"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn render_audit_popup_paints_the_audit_section_body_rows() {
        // Given the audit popup rendered with one excluded entry.
        let (buffer, _chat_log_area, rects) = render_popup_buffer().await;
        let popup = only_popup_rect(&rects);

        // When reading the audit header and event body rows, past the
        // Metadata title, Sent line, and their blank row.
        let audit_header_y = popup.y + 4;
        let body_y = popup.y + 5;
        let row_text = |y: u16| -> String {
            (popup.x..popup.x + popup.width)
                .filter_map(|x| buffer.cell((x, y)).map(|c| c.symbol().to_owned()))
                .collect()
        };
        let audit_header_row = row_text(audit_header_y);
        let body_row = row_text(body_y);

        // Then the header row names the event count and the excluded tool.
        assert!(
            audit_header_row.contains("audit")
                && audit_header_row.contains("1 events")
                && audit_header_row.contains("ForcedExclude"),
            "audit header row at y={audit_header_y}: {audit_header_row:?}"
        );
        // And the body row names the role and decision.
        assert!(
            body_row.contains("user") && body_row.contains("Default"),
            "body row at y={body_y}: {body_row:?}"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn render_audit_popup_paints_vertical_borders_on_every_content_row() {
        // Given the audit popup rendered with one excluded entry.
        let (buffer, _chat_log_area, rects) = render_popup_buffer().await;
        let popup = only_popup_rect(&rects);

        // When probing every row strictly between the top and bottom borders.
        let rows = (popup.y + 1)..(popup.y + popup.height - 1);
        let edges = rows
            .map(|y| {
                let left = buffer.cell((popup.x, y)).map(|c| c.symbol()).unwrap_or("");
                let right = buffer
                    .cell((popup.x + popup.width - 1, y))
                    .map(|c| c.symbol())
                    .unwrap_or("");
                (y, left, right)
            })
            .collect::<Vec<_>>();

        // Then each is bounded by the vertical border glyph on both edges.
        for (y, left, right) in edges {
            assert_eq!(left, "│", "missing left border at ({}, {})", popup.x, y);
            assert_eq!(
                right,
                "│",
                "missing right border at ({}, {})",
                popup.x + popup.width - 1,
                y
            );
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn render_audit_popup_paints_four_rounded_corners() {
        // Given the audit popup rendered into a buffer.
        let (buffer, _chat_log_area, rects) = render_popup_buffer().await;
        let popup = only_popup_rect(&rects);

        let top = popup.y;
        let bottom = popup.y + popup.height - 1;
        let left = popup.x;
        let right = popup.x + popup.width - 1;

        // When reading the four corner cells.
        let corners = [
            ((left, top), buffer.cell((left, top)).map(|c| c.symbol())),
            ((right, top), buffer.cell((right, top)).map(|c| c.symbol())),
            (
                (left, bottom),
                buffer.cell((left, bottom)).map(|c| c.symbol()),
            ),
            (
                (right, bottom),
                buffer.cell((right, bottom)).map(|c| c.symbol()),
            ),
        ];

        // Then the four corners are the rounded border glyphs.
        assert_eq!(corners[0].1, Some("╭"), "top-left corner");
        assert_eq!(corners[1].1, Some("╮"), "top-right corner");
        assert_eq!(corners[2].1, Some("╰"), "bottom-left corner");
        assert_eq!(corners[3].1, Some("╯"), "bottom-right corner");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn render_audit_popup_skips_when_visibility_off() {
        // Given a state with audit popup visibility OFF.
        let mut app = crate::TuiApp::test_builder().build().await;
        let mut entry = ChatEntry::user("hello");
        entry.apply_context_override(ContextOverride::ForcedExclude, ChangeSource::User);
        // the audit-popup cell stays at its default (hidden)
        app.core
            .state
            .write()
            .active_session_mut()
            .push_entry(entry);
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
    #[tokio::test]
    async fn render_audit_popup_skips_when_picker_overlay_active() {
        // Given a state with audit visible BUT a Picker overlay on top.
        let mut app = app_with_audit_visible().await;
        app.core
            .state
            .write()
            .frontend
            .scope_push(FocusScope::Picker {
                kind: jinn_slices::picker_kind::PickerKind::Project,
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
