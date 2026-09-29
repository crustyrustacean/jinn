//! Render tests for the attendant properties popup: styling of the field
//! rows, the hint/footer derivation, and the cursor split between the two
//! popup phases. Rendered through `TestBackend` with a real `RenderFacts`.

#![allow(clippy::expect_used, clippy::panic, reason = "test code")]
#![allow(
    clippy::used_underscore_binding,
    reason = "rstest case labels document intent"
)]

use jinn_attendant_msg::{
    AttendantActivation, AttendantPropertiesState, AttendantTrigger, PropertyField,
    attendant_properties_slot,
};
use jinn_slices::RenderFacts;
use jinn_slices::cell::TypedCell;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::{Position, Rect};
use ratatui::style::Color;
use unicode_segmentation::UnicodeSegmentation;

/// The default theme's plain-text color.
const PRIMARY_TEXT: Color = Color::Rgb(220, 220, 220);

use crate::properties_overlay::{
    attendant_properties_overlay_rect, render_attendant_properties, render_attendant_seed_template,
};

/// Registers the popup's cell on a fresh registry (the cell catalog does
/// this at boot; the test needs only the slot the overlays resolve).
fn slices_with_popup(
    popup: AttendantPropertiesState,
) -> (jinn_slices::Slices, TypedCell<AttendantPropertiesState>) {
    let slices = jinn_slices::Slices::new();
    let cell = slices
        .register(attendant_properties_slot(), popup)
        .expect("unclaimed slot");
    (slices, cell)
}

/// Renders the seed-template editor view (the form plus the text cursor)
/// and returns the terminal, so a test can read where the cursor landed.
fn render_editor(popup: AttendantPropertiesState) -> Terminal<TestBackend> {
    let (slices, _cell) = slices_with_popup(popup);
    let facts = RenderFacts::new(jinn_theme::default_theme(), &slices);
    let area = Rect::new(0, 0, 100, 30);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).expect("terminal");
    terminal
        .draw(|frame| {
            let rect = attendant_properties_overlay_rect(&area).expect("geometry");
            render_attendant_seed_template(frame, rect, &facts);
        })
        .expect("draw");
    terminal
}

/// Renders the properties view into a backend and returns the buffer cells.
fn render_properties(popup: AttendantPropertiesState) -> ratatui::buffer::Buffer {
    let (slices, _cell) = slices_with_popup(popup);
    let facts = RenderFacts::new(jinn_theme::default_theme(), &slices);
    let area = Rect::new(0, 0, 100, 30);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).expect("terminal");
    terminal
        .draw(|frame| {
            let rect = attendant_properties_overlay_rect(&area).expect("geometry");
            render_attendant_properties(frame, rect, &facts);
        })
        .expect("draw");
    terminal.backend().buffer().clone()
}

/// The character at a buffer position.
fn symbol_at(buffer: &ratatui::buffer::Buffer, x: u16, y: u16) -> String {
    buffer[(x, y)].symbol().to_owned()
}

/// The foreground color at a buffer position.
fn fg_at(buffer: &ratatui::buffer::Buffer, x: u16, y: u16) -> Color {
    buffer[(x, y)].fg
}

/// Finds the x position of the first occurrence of `needle` in row `y`.
fn find_in_row(buffer: &ratatui::buffer::Buffer, y: u16, needle: &str) -> Option<u16> {
    // Positions are cell indices measured from the popup's left border, so
    // the row is cut there (not at x=0) to keep offsets screen-aligned.
    let line = row_from_border(buffer, y);
    let byte_index = line.find(needle)?;
    Some(inner_left() + u16::try_from(byte_index).expect("row fits u16"))
}

/// The popup row `y` as text, starting at its left border cell.
fn row_from_border(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
    let start = inner_left();
    (start..buffer.area.width)
        .map(|x| symbol_at(buffer, x, y))
        .collect()
}

/// The x of the popup's left border: the centered rect's own x, not a scan
/// (row text can contain `│` of its own).
fn inner_left() -> u16 {
    let area = Rect::new(0, 0, 100, 30);
    attendant_properties_overlay_rect(&area)
        .expect("geometry")
        .x
}

/// The popup's first body row inside the border, found by scanning for the
/// top border corner `┌` anywhere in the row.
fn body_top(buffer: &ratatui::buffer::Buffer) -> u16 {
    for y in buffer.area.y..buffer.area.height {
        for x in 0..buffer.area.width {
            if symbol_at(buffer, x, y) == "┌" {
                return y + 1;
            }
        }
    }
    panic!("no popup top border found");
}

/// The y of the template draft row: the body row containing `seed template:`.
fn template_row_y(buffer: &ratatui::buffer::Buffer) -> u16 {
    for y in body_top(buffer)..buffer.area.height {
        if find_in_row(buffer, y, "seed template:").is_some() {
            return y;
        }
    }
    panic!("no template row found");
}

/// The popup's bottom border row, found by walking the right border down.
fn template_row_bottom_y(buffer: &ratatui::buffer::Buffer, right_border_x: u16) -> u16 {
    let mut y = body_top(buffer);
    loop {
        let symbol = symbol_at(buffer, right_border_x, y);
        if symbol == "┘" || symbol == "┤" {
            return y;
        }
        y += 1;
        assert!(y < buffer.area.height, "no bottom border found");
    }
}

/// The x of the popup's right border on `y`.
fn right_border_x(buffer: &ratatui::buffer::Buffer, y: u16) -> u16 {
    let mut x = buffer.area.width - 1;
    while x > 0 {
        if symbol_at(buffer, x, y) == "│" {
            return x;
        }
        x -= 1;
    }
    panic!("no right border found");
}

/// An open popup over a fresh attendant, focused as given.
fn popup_focused(focus: PropertyField) -> AttendantPropertiesState {
    AttendantPropertiesState {
        focus,
        pending_trigger: AttendantTrigger::ParentCompleted,
        pending_activation: AttendantActivation::Continue,
        seed_template: jinn_slices::LineInput {
            input: "draft".to_owned(),
            cursor_pos: 5,
        },
        ..AttendantPropertiesState::default()
    }
}

#[rstest::rstest]
#[test]
fn focused_row_marker_and_label_are_yellow_only() {
    // Given a popup focused on the trigger row.
    let buffer = render_properties(popup_focused(PropertyField::Trigger));
    let top = body_top(&buffer);

    // When locating the focused row's marker and label.
    let marker_x = find_in_row(&buffer, top, "▸").expect("focused marker");
    let label_x = find_in_row(&buffer, top, "trigger:").expect("trigger label");

    // Then both are the focus accent.
    assert_eq!(fg_at(&buffer, marker_x, top), Color::Yellow);
    assert_eq!(fg_at(&buffer, label_x, top), Color::Yellow);
    // And the row's choice text is not.
    let choice_x = find_in_row(&buffer, top, "parent-completed").expect("trigger value");
    assert_eq!(
        fg_at(&buffer, choice_x, top),
        Color::LightGreen,
        "the selected choice stays green even on the focused row"
    );
}

#[rstest::rstest]
#[test]
fn unfocused_row_marker_and_label_are_plain() {
    // Given a popup focused on the trigger row.
    let buffer = render_properties(popup_focused(PropertyField::Trigger));
    let top = body_top(&buffer);

    // When locating the unfocused activation row (the trigger's hint line
    // sits between them).
    let activation_y = top + 2;
    let label_x = find_in_row(&buffer, activation_y, "activation:").expect("activation label");

    // Then the label is plain text, not the focus accent.
    assert_eq!(fg_at(&buffer, label_x, activation_y), PRIMARY_TEXT);
}

#[rstest::rstest]
#[test]
fn selected_choice_uses_the_new_green_key() {
    // Given a popup whose pending activation is `continue` (last choice),
    // focused on the template field so no choice row is focused.
    let buffer = render_properties(popup_focused(PropertyField::SeedTemplate));
    let top = body_top(&buffer);
    let activation_y = top + 1; // no hint above: the trigger row is unfocused

    // When locating the three activation choices.
    let seed_x = find_in_row(&buffer, activation_y, "seed").expect("seed");
    let reset_x = find_in_row(&buffer, activation_y, "reset").expect("reset");
    let continue_x = find_in_row(&buffer, activation_y, "continue").expect("continue");

    // Then the selected choice is the attendant option green…
    assert_eq!(fg_at(&buffer, continue_x, activation_y), Color::LightGreen);
    // …and the unselected ones are plain text.
    assert_eq!(fg_at(&buffer, seed_x, activation_y), PRIMARY_TEXT);
    assert_eq!(fg_at(&buffer, reset_x, activation_y), PRIMARY_TEXT);
}

#[rstest::rstest]
#[test]
fn hint_line_renders_only_under_the_focused_row() {
    // Given a popup focused on the activation row.
    let buffer = render_properties(popup_focused(PropertyField::Activation));
    let all_text = |buffer: &ratatui::buffer::Buffer| {
        let mut text = String::new();
        for y in buffer.area.y..buffer.area.height {
            for x in buffer.area.x..buffer.area.width {
                text.push_str(&symbol_at(buffer, x, y));
            }
            text.push('\n');
        }
        text
    };

    // When reading the rendered popup.
    let rendered = all_text(&buffer);

    // Then the activation hint appears once (under its focused row)…
    assert_eq!(rendered.matches("seed pins without dispatching").count(), 1);
    // …and no other field's hint renders.
    assert!(
        !rendered.contains("does this attendant re-run"),
        "unfocused hints must not render"
    );
    assert!(
        !rendered.contains("injected ahead of each run"),
        "unfocused hints must not render"
    );
}

#[rstest::rstest]
#[test]
fn footer_derives_from_the_focused_field() {
    // Given a popup focused on the seed template.
    let buffer = render_properties(popup_focused(PropertyField::SeedTemplate));

    // When reading the rendered popup.
    let mut rendered = String::new();
    for y in buffer.area.y..buffer.area.height {
        for x in buffer.area.x..buffer.area.width {
            rendered.push_str(&symbol_at(&buffer, x, y));
        }
    }

    // Then the template-focused footer shows the `i` edit key…
    assert!(rendered.contains("i edit · j/k field"));
    // …and no pick keys, which only a choice row uses.
    assert!(!rendered.contains("h/l pick"));
}

#[rstest::rstest]
#[test]
fn footer_lists_pick_keys_on_a_choice_field() {
    // Given a popup focused on the trigger.
    let buffer = render_properties(popup_focused(PropertyField::Trigger));

    // When reading the rendered popup.
    let mut rendered = String::new();
    for y in buffer.area.y..buffer.area.height {
        for x in buffer.area.x..buffer.area.width {
            rendered.push_str(&symbol_at(&buffer, x, y));
        }
    }

    // Then the footer shows the pick keys.
    assert!(rendered.contains("h/l pick · j/k field"));
}

#[rstest::rstest]
#[test]
fn template_row_shows_truncated_pending_text() {
    // Given a template draft far wider than the popup.
    let mut popup = popup_focused(PropertyField::SeedTemplate);
    popup.seed_template.input = "x".repeat(500);

    // When rendering the properties view.
    let buffer = render_properties(popup);
    let template_y = template_row_y(&buffer);

    // Then the draft is cut to the popup's inner width: the text stops
    // before the border column.
    let right_border_x = right_border_x(&buffer, template_y);
    let fill_end = find_last_non_space_before(&buffer, template_y, right_border_x);
    assert!(
        fill_end < right_border_x,
        "the truncated draft stops inside the border (fill ended at {fill_end}, border at {right_border_x})"
    );
    // And the draft did not spill outside the popup: the border column
    // holds a border glyph down to the bottom corner.
    let bottom_y = template_row_bottom_y(&buffer, right_border_x);
    for y in template_y..=bottom_y {
        let symbol = symbol_at(&buffer, right_border_x, y);
        assert!(
            symbol == "│" || symbol == "┐" || symbol == "┘",
            "border intact at y={y}, got {symbol:?}"
        );
    }
}

/// The last x with a non-space symbol strictly before `border_x`.
fn find_last_non_space_before(buffer: &ratatui::buffer::Buffer, y: u16, border_x: u16) -> u16 {
    let mut x = border_x;
    while x > 0 {
        x -= 1;
        if symbol_at(buffer, x, y) != " " {
            return x;
        }
    }
    0
}

#[rstest::rstest]
#[test]
fn template_truncation_never_splits_a_grapheme() {
    // Given a draft whose tail is a multi-byte grapheme cluster (a flag,
    // which unicode-segmentation keeps whole).
    let mut popup = popup_focused(PropertyField::SeedTemplate);
    let long_prefix = "x".repeat(200);
    popup.seed_template.input = format!("{long_prefix}🇺🇳tail");

    // When rendering.
    let buffer = render_properties(popup);
    let template_y = template_row_y(&buffer);
    for x in buffer.area.x..buffer.area.width {
        let symbol = symbol_at(&buffer, x, template_y);
        assert!(
            symbol != "\u{FFFD}" && !symbol.contains('\u{FFFD}'),
            "grapheme was split at x={x}: {symbol:?}"
        );
    }
}

#[rstest::rstest]
#[test]
fn properties_view_sets_no_terminal_cursor() {
    // Given the properties scope's view over an open popup.
    let (slices, _cell) = slices_with_popup(popup_focused(PropertyField::SeedTemplate));
    let facts = RenderFacts::new(jinn_theme::default_theme(), &slices);
    let area = Rect::new(0, 0, 100, 30);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).expect("terminal");

    // When rendering.
    terminal
        .draw(|frame| {
            let rect = attendant_properties_overlay_rect(&area).expect("geometry");
            render_attendant_properties(frame, rect, &facts);
        })
        .expect("draw");

    // Then no cursor position was set.
    assert_eq!(
        terminal.get_cursor_position().expect("cursor"),
        Position::ORIGIN,
        "the navigation-only form must not show a text cursor"
    );
}

#[rstest::rstest]
#[test]
fn editor_view_places_the_cursor_in_the_draft() {
    // Given the editor scope's view over a popup whose draft is "draft",
    // with the cursor at its end.
    let mut popup = popup_focused(PropertyField::SeedTemplate);
    popup.seed_template.cursor_pos = popup.seed_template.input.len();

    // When rendering.
    let mut terminal = render_editor(popup);

    // Then the cursor sits on the template row — with focus on the seed
    // template field, no hint line renders above it, so the draft is the
    // third body row (trigger, activation, template) — and exactly one
    // cell past the draft's last grapheme.
    let cursor = terminal.get_cursor_position().expect("cursor");
    let template_y = 7 + 1 + 2; // top border 7, body rows 8.., template is body row 2
    assert_eq!(cursor.y, template_y, "cursor rests on the template row");
    let buffer = terminal.backend().buffer();
    assert_eq!(
        symbol_at(buffer, cursor.x.saturating_sub(1), template_y),
        "t",
        "the cell before the cursor is the draft's last grapheme"
    );
    // And the marker and label precede the value on the same row.
    assert!(
        find_in_row(buffer, template_y, "▸ seed template:  draft").is_some(),
        "the value follows the marker and label"
    );
}

/// A draft longer than the row scrolls so the cursor stays on screen.
#[rstest::rstest]
#[case(0, "the draft's tail: the window ends at the cursor")]
#[case(1, "mid-draft: the window follows the cursor left")]
#[case(6, "the draft's start: the window returns to the head")]
fn editor_window_follows_the_cursor_in_a_long_draft(
    #[case] cursor_from_end: usize,
    #[case] _about: &str,
) {
    // Given a 60-grapheme draft in a row that shows far fewer.
    let mut popup = popup_focused(PropertyField::SeedTemplate);
    let long = "abcdefghij".repeat(6); // 60 graphemes
    popup.seed_template.input = long.clone();
    // Place the cursor `cursor_from_end` graphemes before the end.
    let cursor_index = long.graphemes(true).count() - cursor_from_end;
    popup.seed_template.cursor_pos = {
        let before_cursor = long.graphemes(true).take(cursor_index).collect::<String>();
        before_cursor.len()
    };

    // When rendering.
    let mut terminal = render_editor(popup);
    let cursor = terminal.get_cursor_position().expect("cursor");
    let buffer = terminal.backend().buffer();
    let template_y = 7 + 1 + 2;

    // Then the cursor rests immediately after a visible draft grapheme —
    // never floating over the blank tail of the row, which is what a
    // truncated draft drawn from its head and an un-scrolled cursor
    // together produced.
    let before_cursor = symbol_at(buffer, cursor.x.saturating_sub(1), template_y);
    assert!(
        before_cursor.chars().all(|c| c.is_ascii_alphanumeric()),
        "the cell before the cursor holds draft text, not blank space (got {before_cursor:?})"
    );
}

/// A wide (double-cell) draft keeps the cursor glued to its own text.
#[rstest::rstest]
#[case(0, "the cursor at the end of the draft")]
#[case(3, "the cursor partway back into the draft")]
fn wide_graphemes_keep_the_cursor_on_its_own_text(
    #[case] cursor_from_end: usize,
    #[case] _about: &str,
) {
    // Given a CJK draft — every grapheme two cells wide, so the row holds
    // half as many graphemes as cells — with the cursor set back from the end.
    let mut popup = popup_focused(PropertyField::SeedTemplate);
    let cjk = ".attendant.report".repeat(20);
    popup.seed_template.input = cjk.clone();
    let cursor_index = cjk.graphemes(true).count() - cursor_from_end;
    popup.seed_template.cursor_pos = {
        let before = cjk.graphemes(true).take(cursor_index).collect::<String>();
        before.len()
    };

    // When rendering.
    let mut terminal = render_editor(popup);
    let cursor = terminal.get_cursor_position().expect("cursor");
    let buffer = terminal.backend().buffer();
    let template_y = 7 + 1 + 2;

    // Then the cell under the cursor is a real (blank) cursor cell within
    // the popup, and the text before it ends where the cursor is.
    let right_border_x = right_border_x(buffer, template_y);
    assert!(
        cursor.x < right_border_x,
        "the cursor stays inside the popup (cursor x={}, border x={right_border_x})",
        cursor.x
    );
    // And the cell just left of the cursor holds a draft grapheme, never
    // the border or a stray space mid-draft.
    let before_cursor = symbol_at(buffer, cursor.x.saturating_sub(1), template_y);
    assert_ne!(before_cursor, "│", "the cursor is not on top of the border");
}

/// A draft far wider than the row never spills past the popup border.
#[rstest::rstest]
#[case(PropertyField::SeedTemplate, "the focused row's wide marker")]
#[case(PropertyField::Activation, "an unfocused row's plain marker")]
fn template_window_never_overwrites_the_popup_border(
    #[case] focus: PropertyField,
    #[case] _about: &str,
) {
    // Given a draft much longer than the row, on a focused and an
    // unfocused template row.
    let mut popup = popup_focused(focus);
    popup.seed_template.input = "x".repeat(400);
    popup.seed_template.cursor_pos = popup.seed_template.input.len();

    // When rendering.
    let buffer = render_properties(popup);
    let template_y = template_row_y(&buffer);

    // Then the row's last cells are the border and the cell the cursor may
    // rest on — the window is clipped to the row, not the popup.
    let right_border_x = right_border_x(&buffer, template_y);
    assert_eq!(
        symbol_at(&buffer, right_border_x, template_y),
        "│",
        "the right border survives a draft wider than the row"
    );
    assert_eq!(
        symbol_at(&buffer, right_border_x.saturating_sub(1), template_y),
        " ",
        "the cell before the border is left for the cursor"
    );
}

#[rstest::rstest]
#[test]
fn color_tests_style_map_counts_the_new_key() {
    // Given the default theme.

    // When building its style map.
    let map = jinn_theme::default_theme().style_map();

    // When reading the entry for the new key.
    let entry = map.get("attendant_option_active");

    // Then it is present and resolves to the active green.
    assert_eq!(
        entry.map(|style| style.fg),
        Some(Some(Color::LightGreen)),
        "attendant_option_active must be in the style map"
    );
}

/// The theme's raw color for the key equals the picker's palette green.
#[rstest::rstest]
#[test]
fn theme_key_defaults_to_the_age_fresh_green() {
    // Given the default theme's `age_fresh` color.
    let theme = jinn_theme::default_theme();

    // When reading both colors.
    let fresh: Color = theme.age_fresh;
    let option: Color = theme.attendant_option_active;

    // Then they match by default.
    assert_eq!(fresh, option);
}
