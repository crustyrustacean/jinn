//! The skill picker's overlay rendering.
//!
//! A slice-owned picker renders from the slice's own cell. The render
//! context deliberately never sees `AppState` (see `RenderFacts`), so this
//! reads `SkillPickerState` straight off the registered slot and drives the
//! preview selection widget itself — no `PickerHost`, no kernel borrow, and
//! no picker-kind lookup in the kernel.

use jinn_selection_widget::PreviewSelectionWidget;
use jinn_skills_msg::SkillPickerState;
use jinn_slices::RenderFacts;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;

/// The popup rect for the skill picker: the shared selection-widget popup.
#[must_use]
pub fn skill_picker_overlay_rect(area: &Rect) -> Option<Rect> {
    Some(jinn_selection_widget::compute_popup_rect(*area))
}

/// The colors the skill picker draws with.
///
/// Mirrors the widget's defaults, matching what the kernel-side spec render
/// produced: the chrome fields are unthemed, the accent fields come from the
/// active theme.
#[must_use]
pub fn skill_picker_palette(theme: &jinn_theme::Theme) -> jinn_picker::Palette {
    jinn_picker::Palette {
        border: ratatui::style::Color::DarkGray,
        filter_text: ratatui::style::Color::White,
        separator: ratatui::style::Color::DarkGray,
        footer: ratatui::style::Color::DarkGray,
        highlight_bg: ratatui::style::Color::DarkGray,
        muted_text: theme.muted_text,
        accent_action: theme.accent_action,
        popup_title: theme.popup_title,
        primary_text: theme.primary_text,
    }
}

/// The skill picker's declared binds, in footer order.
///
/// Sourced from the route rows the slice actually attaches, so the footer can
/// never advertise a key the picker does not bind.
#[must_use]
pub fn skill_picker_binds() -> Vec<jinn_picker::BindRow> {
    crate::skill_picker_routes::SKILL_PICKER_BINDINGS
        .iter()
        .map(|(notation, label)| jinn_picker::BindRow {
            notation,
            label,
            category_hint: "input",
        })
        .collect()
}

/// Draws the skill picker popup for one frame.
pub fn render_skill_picker(frame: &mut Frame<'_>, area: Rect, facts: &RenderFacts) {
    let Some(cell) = facts.slices.reader(&jinn_skills_msg::skill_picker_slot()) else {
        return;
    };

    // Measure the popup's result rows and publish them for the navigation keys,
    // which need a real row count to keep the highlight on screen. This is the
    // slice-owned equivalent of the kernel's per-frame viewport write.
    let measured = crate::skill_picker_viewport::results_viewport(area);
    cell.update(|state: &mut SkillPickerState| state.results_viewport = measured);

    let guard = cell.read();
    let state: &SkillPickerState = &guard;

    let theme = &facts.theme;
    let palette = skill_picker_palette(theme);
    let binds = skill_picker_binds();
    let keybind = jinn_picker::keybind_line(&binds, jinn_picker::Tail::Standard, &palette);
    let footers = vec![Line::from(String::new()), Line::from(keybind.0)];

    let widget = PreviewSelectionWidget::new(&state.selection)
        .title(Line::from(" Skills "))
        .title_style(Style::default().fg(theme.popup_title))
        .footers(footers)
        .colors(palette.selection_colors())
        .preview_scroll(state.preview_scroll);

    let cache = state.preview_cache.clone();
    widget.preview_cache(cache.as_ref()).render(frame, area);
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        reason = "test module, panics are acceptable"
    )]
    use super::*;
    use jinn_slices::OverlayViews;
    use jinn_slices::SliceHost;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// The load-bearing test for a slice-owned picker: the render pass must
    /// draw it. The kernel dispatches a `FocusScope::Dynamic` picker through
    /// the generic overlay path, so this exercises geometry + view + cell
    /// with no kernel borrow at all.
    #[rstest::rstest]
    #[tokio::test]
    async fn render_draws_the_skill_picker_from_its_own_cell() {
        // Given an activated skills slice holding one skill.
        let slices = jinn_slices::Slices::new();
        let mut viewport = jinn_slices::view::Viewport::new();
        let overlay_views = OverlayViews::new();
        let key_routes = jinn_slices::KeyRoutes::new();
        let services = jinn_kernel::Services::new_fake().await;
        let mut host = SliceHost::new(
            &slices,
            &mut viewport,
            &overlay_views,
            &key_routes,
            &services.trouper_system,
        );
        crate::activate(&mut host);
        slices
            .reader::<jinn_skills_msg::SkillPickerState>(&jinn_skills_msg::skill_picker_slot())
            .expect("cell registered")
            .update(|state: &mut jinn_skills_msg::SkillPickerState| {
                state
                    .selection
                    .set_items(jinn_picker::make_items_with_hooks(
                        vec![jinn_skills_msg::SkillEntry {
                            name: "web-coder".to_owned(),
                            description: "writes web code".to_owned(),
                            body: "# Body".to_owned(),
                            enabled: true,
                            source: jinn_skills_msg::SkillSource::Global,
                            theme: jinn_theme::default_theme(),
                        }],
                        jinn_picker::PickerItemHooks::new()
                            .row(jinn_skills_msg::skill_row)
                            .search(|e: &jinn_skills_msg::SkillEntry| e.name.clone()),
                    ));
            });
        let facts = jinn_slices::RenderFacts::new(jinn_theme::default_theme(), &slices);

        // When the overlay view renders the picker.
        let area = Rect::new(0, 0, 100, 30);
        let mut terminal =
            Terminal::new(TestBackend::new(area.width, area.height)).expect("terminal");
        terminal
            .draw(|frame| {
                let popup = skill_picker_overlay_rect(&area).expect("geometry fn yields a rect");
                render_skill_picker(frame, popup, &facts);
            })
            .expect("draw");

        // Then the skill's name is on screen — the picker draws itself.
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(
            rendered.contains("web-coder"),
            "slice-owned picker must draw its rows; got {rendered:?}"
        );
    }

    /// The overlay must be discoverable by identity alone: the render pass
    /// looks it up through the registry with no picker-specific branch.
    #[rstest::rstest]
    #[tokio::test]
    async fn registered_overlay_resolves_by_scope_identity() {
        // Given an activated skills slice.
        let slices = jinn_slices::Slices::new();
        let mut viewport = jinn_slices::view::Viewport::new();
        let overlay_views = OverlayViews::new();
        let key_routes = jinn_slices::KeyRoutes::new();
        let services = jinn_kernel::Services::new_fake().await;
        let mut host = SliceHost::new(
            &slices,
            &mut viewport,
            &overlay_views,
            &key_routes,
            &services.trouper_system,
        );
        crate::activate(&mut host);
        let scope = crate::skill_picker_scope();

        // When the render pass resolves the scope's overlay and view.
        let overlay = slices.overlay(&scope);
        let view = overlay_views.view(&scope);
        let area = Rect::new(0, 0, 100, 30);

        // Then both are present and the geometry yields a popup rect.
        assert!(overlay.is_some(), "overlay geometry must be registered");
        assert!(view.is_some(), "overlay view must be registered");
        let rect = overlay.expect("overlay")(&area).expect("rect");
        assert!(rect.width > 0 && rect.height > 0);
    }
}
