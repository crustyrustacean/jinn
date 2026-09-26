//! Picker overlay rendering - dispatches to domain-specific picker renderers.

use jinn_domain::RenderCtx;
use ratatui::Frame;
use ratatui::layout::Rect;

/// Renders the active picker overlay, dispatching on [`PickerKind`].
pub(super) fn render_picker(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    // Every picker kind is spec-driven: render through the registry. With
    // an empty registry (test seams) there is nothing to draw. `None`
    // (no picker scope) is also a no-op here.
    if let Some(kind) = ctx.state.frontend.picker_kind()
        && let Some(id) = jinn_picker::spec_id_for_kind(&kind)
        && let Some(spec) = ctx.pickers.get(id)
    {
        let host = jinn_domain::feat::picker::host_impl::AppStateRenderHost::new(ctx.state);
        let outcome = spec.render(frame, area, &host);
        // There is no fallback renderer, so a spec that could not draw
        // leaves an empty popup on screen. That is a wiring defect — the
        // spec's storage shape does not match what the host lends — and it
        // is otherwise completely silent, so say so loudly.
        if !outcome.drew() {
            tracing::error!(
                picker = spec.id().as_str(),
                widget = ?spec.widget_kind(),
                "picker spec drew nothing: the host lent no storage it could drive",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "test code, panics are acceptable"
    )]
    use jinn_domain::AppState;
    use jinn_domain::PickerKind;
    use jinn_selection_widget::compute_popup_rect;
    use jinn_slices::FocusScope;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;

    #[rstest::rstest]
    fn larger_terminal_gets_taller_popup() {
        // Given two terminal sizes.
        let small_area = Rect::new(0, 0, 80, 24);
        let large_area = Rect::new(0, 0, 80, 42);

        // When computing popup rects.
        let small_popup = compute_popup_rect(small_area);
        let large_popup = compute_popup_rect(large_area);

        // Then the larger terminal gets a taller popup.
        assert!(large_popup.height > small_popup.height);
    }

    #[rstest::rstest]
    fn small_terminal_uses_75_percent_height() {
        // Given two terminal sizes.
        let small_area = Rect::new(0, 0, 80, 24);
        let large_area = Rect::new(0, 0, 80, 42);

        // When computing popup rects.
        let small_popup = compute_popup_rect(small_area);
        let _large_popup = compute_popup_rect(large_area);

        // Then the small terminal popup uses 75% of height + 4 rows of chrome.
        // floor(24 * 0.75) = 18, min(18 + 4, 24) = 22.
        assert_eq!(small_popup.height, 22);
    }

    /// Each picker kind must draw exactly the number of footer rows its spec
    /// declares via `bottom_rows()`. This is the drift-prevention backstop for
    /// the picker viewport measurement: if a render site ever adds or drops a
    /// footer without updating the spec, the geometry helper would reserve the
    /// wrong number of rows and the cursor could drift off-screen. With an
    /// empty item list, the results area is blank, so the consecutive
    /// non-blank rows at the bottom of the popup's inner area equal the footer
    /// count actually drawn.
    #[rstest::rstest]
    #[case::project(PickerKind::Project)]
    fn picker_draws_footer_rows_matching_kind_declaration(#[case] kind: PickerKind) {
        // Given a picker scope of this kind with the default (empty) state,
        // and the domain's picker registry.
        let state = AppState::default_with_scope_focus();
        state.frontend.scope_push(FocusScope::Picker { kind });
        let pickers = jinn_picker_specs::build_picker_registry();

        // When rendering the picker overlay.
        let area = Rect::new(0, 0, 100, 30);
        let mut terminal =
            Terminal::new(TestBackend::new(area.width, area.height)).expect("terminal");
        terminal
            .draw(|frame| {
                let slices = jinn_slices::Slices::new();
                let views = jinn_slices::OverlayViews::new();
                let ctx =
                    jinn_domain::RenderCtx::new(&state, &slices, &views).with_pickers(&pickers);
                super::render_picker(frame, area, &ctx);
            })
            .expect("draw");

        // Then the number of footer rows actually drawn equals the kind's declaration.
        let popup = compute_popup_rect(area);
        // Inner popup area excludes the border.
        let inner_top = popup.y + 1;
        let inner_bottom = popup.y + popup.height.saturating_sub(2);
        let inner_x_start = popup.x + 1;
        let inner_x_end = popup.x + popup.width.saturating_sub(1);

        let buffer = terminal.backend().buffer();
        let row_is_blank = |y: u16| -> bool {
            (inner_x_start..inner_x_end).all(|x| buffer[(x, y)].symbol().trim().is_empty())
        };

        // Count consecutive non-blank rows climbing up from the bottom of the
        // popup. With an empty results list this is exactly the footer block.
        let mut drawn_footer_rows = 0u16;
        for y in (inner_top..=inner_bottom).rev() {
            if row_is_blank(y) {
                break;
            }
            drawn_footer_rows += 1;
        }

        // The declared footer count is spec-owned (every kind has a spec).
        let declared = jinn_picker::spec_id_for_kind(&kind)
            .and_then(|id| pickers.get(id))
            .map_or(1, |spec| spec.bottom_rows());

        assert_eq!(
            drawn_footer_rows, declared,
            "picker {kind} draws {drawn_footer_rows} footer rows but declares {declared}",
        );
    }

    /// Every registered spec must draw through the real render host.
    ///
    /// A spec whose lent storage is the wrong shape — a tree spec handed a
    /// flat `SelectionState`, or a flat spec with no compatible lend — draws
    /// *nothing at all*: no error, no log, just an empty frame. This walks
    /// every spec the app actually registers so that failure mode is caught
    /// here rather than on screen.
    #[rstest::rstest]
    #[test]
    fn every_registered_spec_renders_a_non_empty_frame() {
        // Given the real picker registry and default state.
        let state = AppState::default_with_scope_focus();
        let pickers = jinn_picker_specs::build_picker_registry();
        let area = Rect::new(0, 0, 100, 30);

        // When rendering each registered spec through the real render host.
        let blank_specs: Vec<&str> = pickers
            .ids()
            .into_iter()
            .filter(|id| {
                let spec = pickers.get(id).expect("registered spec");
                let mut terminal =
                    Terminal::new(TestBackend::new(area.width, area.height)).expect("terminal");
                terminal
                    .draw(|frame| {
                        let host =
                            jinn_domain::feat::picker::host_impl::AppStateRenderHost::new(&state);
                        spec.render(frame, area, &host);
                    })
                    .expect("draw");
                let rendered: String = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(ratatui::buffer::Cell::symbol)
                    .collect();
                rendered.trim().is_empty()
            })
            .collect();

        // Then no spec renders an empty frame.
        assert!(
            blank_specs.is_empty(),
            "specs that rendered nothing: {blank_specs:?}"
        );
    }
}
