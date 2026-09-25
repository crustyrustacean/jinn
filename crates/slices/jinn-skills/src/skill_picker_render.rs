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
pub const SKILL_PICKER_BINDS: &[jinn_picker::BindRow] = &[jinn_picker::BindRow {
    notation: "<tab>",
    label: "toggle",
    category_hint: "input",
}];

/// Draws the skill picker popup for one frame.
pub fn render_skill_picker(frame: &mut Frame<'_>, area: Rect, facts: &RenderFacts) {
    let Some(cell) = facts.slices.reader(&jinn_skills_msg::skill_picker_slot()) else {
        return;
    };
    let guard = cell.read();
    let state: &SkillPickerState = &guard;

    let theme = &facts.theme;
    let palette = skill_picker_palette(theme);
    let keybind =
        jinn_picker::keybind_line(SKILL_PICKER_BINDS, jinn_picker::Tail::Standard, &palette);
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
