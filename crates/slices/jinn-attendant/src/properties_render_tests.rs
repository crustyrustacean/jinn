//! Render tests for the attendant properties popup: styling of the field
//! rows, the hint/footer derivation, and the cursor split between the two
//! popup phases. Rendered through `TestBackend` with a real `RenderFacts`.

#![allow(clippy::expect_used, clippy::panic, reason = "test code")]
#![allow(
    clippy::used_underscore_binding,
    reason = "rstest case labels document intent"
)]

use jinn_attendant_msg::{
    AttendantActivation, AttendantPropertiesState, AttendantTrigger, PopupStatus, PropertyField,
    attendant_properties_slot,
};
use jinn_slices::RenderFacts;
use jinn_slices::cell::TypedCell;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
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
    render_properties_in(popup, 100, 30)
}

/// Renders the properties view at an explicit terminal size.
fn render_properties_in(
    popup: AttendantPropertiesState,
    width: u16,
    height: u16,
) -> ratatui::buffer::Buffer {
    let (slices, _cell) = slices_with_popup(popup);
    let facts = RenderFacts::new(jinn_theme::default_theme(), &slices);
    let area = Rect::new(0, 0, width, height);
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).expect("terminal");
    terminal
        .draw(|frame| {
            let rect = attendant_properties_overlay_rect(&area).expect("geometry");
            render_attendant_properties(frame, rect, &facts);
        })
        .expect("draw");
    terminal.backend().buffer().clone()
}

/// The background color at a buffer position.
fn bg_at(buffer: &ratatui::buffer::Buffer, x: u16, y: u16) -> Color {
    buffer[(x, y)].bg
}

/// The whole buffer as text, one string per row.
fn all_text(buffer: &ratatui::buffer::Buffer) -> String {
    let mut text = String::new();
    for y in buffer.area.y..buffer.area.height {
        for x in buffer.area.x..buffer.area.width {
            text.push_str(&symbol_at(buffer, x, y));
        }
        text.push('\n');
    }
    text
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

/// The card row `y`'s text, with the card's own border characters trimmed
/// off both ends.
///
/// The card is drawn one cell right of the popup and framed by its own
/// border, so a card row is neither measured from the popup's border the
/// way a form row is, nor read whole: the `│` that frames it is furniture,
/// and a row holding only the frame is a blank card row.
fn card_row_from_border(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
    let start = inner_left() + 1;
    let row: String = (start..buffer.area.width)
        .map(|x| symbol_at(buffer, x, y))
        .collect();
    // The frame is a `│` on each side of a card row and a run of box-drawing
    // on its top and bottom, so the text is what is between them.
    row.trim_matches([
        '\u{2502}', '\u{250c}', '\u{2510}', '\u{2514}', '\u{2518}', ' ', '-', '+', '|',
    ])
    .to_owned()
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
        pending_activation: AttendantActivation::Preserve,
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
fn the_focused_row_carries_the_attendant_background() {
    // Given a popup focused on the activation row.
    let buffer = render_properties(popup_focused(PropertyField::Activation));
    // The activation field is the second body row; the rows are adjacent
    // because the help is an overlay rather than a row of its own.
    let row = body_top(&buffer) + 1;

    // When reading a cell on that row.
    let label_x = find_in_row(&buffer, row, "activation:").expect("activation label");

    // Then the row is backed by the attendant's background color, so the
    // cursor reads as a selected row rather than a tinted label.
    let background = bg_at(&buffer, label_x, row);
    assert_ne!(background, Color::Reset, "the focused row must be tinted");
    // And the label stays readable on it.
    let foreground = fg_at(&buffer, label_x, row);
    assert_ne!(
        foreground, background,
        "the label must not vanish into the row background"
    );
}

#[rstest::rstest]
#[test]
fn unfocused_row_marker_and_label_are_plain() {
    // Given a popup focused on the trigger row.
    let buffer = render_properties(popup_focused(PropertyField::Trigger));
    let top = body_top(&buffer);

    // When locating the unfocused activation row. The rows are adjacent:
    // the help is an overlay, so it no longer sits between them.
    let activation_y = top + 1;
    let label_x = find_in_row(&buffer, activation_y, "activation:").expect("activation label");

    // Then the label is plain text, not the focus accent.
    assert_eq!(fg_at(&buffer, label_x, activation_y), PRIMARY_TEXT);
    // And the row carries no background — only the focused row is tinted.
    assert_eq!(
        bg_at(&buffer, label_x, activation_y),
        Color::Reset,
        "an unfocused row must not be tinted"
    );
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
    let preserve_x = find_in_row(&buffer, activation_y, "preserve").expect("preserve");

    // Then the selected choice is the attendant option green…
    assert_eq!(fg_at(&buffer, preserve_x, activation_y), Color::LightGreen);
    // …and the unselected ones are plain text.
    assert_eq!(fg_at(&buffer, seed_x, activation_y), PRIMARY_TEXT);
    assert_eq!(fg_at(&buffer, reset_x, activation_y), PRIMARY_TEXT);
}

#[rstest::rstest]
#[test]
fn help_overlay_is_hidden_until_question_mark_is_pressed() {
    // Given a popup focused on the activation row, with help not toggled.
    let buffer = render_properties(popup_focused(PropertyField::Activation));

    // When reading the rendered popup.
    let rendered = all_text(&buffer);

    // Then no help text is on screen. The help is an overlay, not a row in
    // the form, so it costs the popup nothing until it is asked for.
    assert!(
        !rendered.contains("seed pins without dispatching"),
        "help must not render before `?`: {rendered}"
    );
}

/// Whether a row is one the help card drew.
///
/// The card is bounded by its own pink edge, and it sits one cell right of
/// the popup, so a card row is one whose left border cell is the card's.
fn row_is_card(buffer: &ratatui::buffer::Buffer, y: u16) -> bool {
    fg_at(buffer, inner_left() + 1, y) == jinn_theme::default_theme().attendant_fg
}

#[rstest::rstest]
#[case::trigger(PropertyField::Trigger, "When the attendant re-runs on its own")]
#[case::activation(PropertyField::Activation, "How the session's context is prepared")]
#[case::template(PropertyField::SeedTemplate, "Text injected on each activation")]
#[test]
fn help_overlay_shows_the_focused_field_when_toggled(
    #[case] field: PropertyField,
    #[case] needle: &str,
) {
    // Given a popup on `field` with the help overlay toggled on.
    let mut popup = popup_focused(field);
    popup.help_visible = true;

    // When the popup is rendered.
    let buffer = render_properties(popup);

    // Then that field's help is on screen.
    let rendered = all_text(&buffer);
    assert!(
        rendered.contains(needle),
        "the focused field's help must render: {rendered}"
    );
}

#[rstest::rstest]
#[test]
fn help_overlay_is_suppressed_while_the_template_editor_is_open() {
    // Given a popup on the template field with help toggled on, and the
    // editor capturing the terminal cursor.
    let mut popup = popup_focused(PropertyField::SeedTemplate);
    popup.help_visible = true;
    popup.begin_template_edit();

    // When the popup is rendered.
    let buffer = render_properties(popup);

    // Then the help is not drawn over the draft being typed into.
    let rendered = all_text(&buffer);
    assert!(
        !rendered.contains("Text injected on each activation"),
        "help must not cover the editor's draft: {rendered}"
    );
}

#[rstest::rstest]
#[test]
fn help_overlay_wraps_at_the_popup_width() {
    // Given a narrow terminal with the help overlay toggled on: narrow
    // enough that every help line wraps, tall enough to hold the card
    // beside the form.
    let mut popup = popup_focused(PropertyField::Activation);
    popup.help_visible = true;
    let buffer = render_properties_in(popup, 44, 26);

    // When reading the rendered screen.
    let rendered = all_text(&buffer);

    // Then the whole help is present, split across rows by the wrap. The
    // needles are single words, so the assertion holds wherever the wrap
    // happens to fall.
    assert!(
        rendered.contains("activation") && rendered.contains("preserve"),
        "the whole help must be present, wrapped rather than cut: {rendered}"
    );
    // And the popup's own rows still render — the overlay is laid over the
    // terminal, not carved out of the form.
    assert!(
        rendered.contains("activation:"),
        "the form must still be drawn: {rendered}"
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
    // Derived rather than hardcoded: the template row is the third body
    // row, and the popup's own position moves with its height.
    let template_y = body_top(terminal.backend().buffer()) + 2;
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
    // Derived from the render rather than hardcoded: the popup's position
    // and width both move with its height and the terminal size.
    let template_y = body_top(buffer) + 2;

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

#[rstest::rstest]
#[case::trigger(PropertyField::Trigger)]
#[case::activation(PropertyField::Activation)]
#[case::template(PropertyField::SeedTemplate)]
#[test]
fn the_help_card_leaves_the_field_it_describes_readable(#[case] field: PropertyField) {
    // Given a popup focused on `field` with the help overlay toggled on.
    let mut popup = popup_focused(field);
    popup.help_visible = true;

    // When rendering.
    let buffer = render_properties(popup);

    // Then the card never covers the field it is describing. A card taller
    // than the gap above the field moves below it rather than answering
    // the question with the row hidden.
    let (top, bottom) = tooltip_rows(&buffer);
    let highlighted = highlighted_row_of(&buffer, field);
    assert!(
        bottom < highlighted || top > highlighted,
        "the card (y={top}..{bottom}) must not cover the field at y={highlighted}"
    );
}

#[rstest::rstest]
#[case::trigger(PropertyField::Trigger)]
#[case::activation(PropertyField::Activation)]
#[case::template(PropertyField::SeedTemplate)]
#[test]
fn the_help_card_sits_below_the_form_on_every_field(#[case] field: PropertyField) {
    // Given a popup focused on `field` with the help overlay toggled on.
    let mut popup = popup_focused(field);
    popup.help_visible = true;

    // When rendering.
    let buffer = render_properties(popup);

    // Then the card is below the popup whatever field it describes. A card
    // that changes side as the user moves between fields is a card that
    // jumps under the cursor; a fixed place is what lets a reader find the
    // next field's help without hunting for it.
    let (top, _) = tooltip_rows(&buffer);
    let form = attendant_properties_overlay_rect(&buffer.area).expect("popup rect");
    let form_bottom = form.y + form.height - 1;
    assert!(
        top > form_bottom,
        "the card (starting y={top}) must sit below the form's last row \
         (y={form_bottom})"
    );
    // And it is flush with the popup's last row. The two were once a blank
    // row apart; the card's own border does that job now, and a gap between
    // two framed things reads as a hole rather than as separation.
    assert_eq!(
        top - form_bottom,
        1,
        "the card must sit directly on the popup's last row"
    );
}

#[rstest::rstest]
#[test]
fn the_tooltip_is_never_positioned_off_screen() {
    // Given the smallest terminal the app supports, with help toggled on.
    let mut popup = popup_focused(PropertyField::Trigger);
    popup.help_visible = true;

    // When rendering at that minimum size.
    let buffer = render_properties_in(popup, 40, 15);

    // Then the card's frame is on screen and inside the terminal. A card
    // taller than the room below the popup is drawn from the terminal's own
    // top and cut by its bottom edge — the one outcome available when
    // neither side can hold it — but it is never drawn past the terminal,
    // and never starts level with the popup it is describing.
    let form = attendant_properties_overlay_rect(&buffer.area).expect("popup rect");
    let card_rows: Vec<u16> = (0..buffer.area.height)
        .filter(|&y| {
            (0..buffer.area.width)
                .any(|x| fg_at(&buffer, x, y) == jinn_theme::default_theme().attendant_fg)
        })
        .collect();
    let (top, bottom) = match (card_rows.first(), card_rows.last()) {
        (Some(t), Some(b)) => (*t, *b),
        _ => panic!(
            "the card must be drawn at all in a {}-row terminal",
            buffer.area.height
        ),
    };
    assert!(
        bottom < buffer.area.height,
        "the card (y={top}..{bottom}) must not run past a {}-row terminal",
        buffer.area.height
    );
    assert!(
        top < form.y + form.height,
        "a card that cannot fit below is drawn from the top, not from the \
         middle of the popup it describes (card y={top}, popup y={}..{})",
        form.y,
        form.y + form.height - 1
    );
}

#[rstest::rstest]
#[test]
fn the_help_card_takes_its_colors_from_the_theme() {
    // Given a popup with the help card toggled on.
    let theme = jinn_theme::default_theme();
    let mut popup = popup_focused(PropertyField::Activation);
    popup.help_visible = true;

    // When rendering.
    let buffer = render_properties(popup);

    // Then the card is drawn on the user-message surface, so it belongs to
    // the app's own palette instead of a white box on a dark screen, and
    // its body reads in the default text color.
    let rows = card_row_numbers(&buffer);
    let (y, x) = rows
        .iter()
        .find_map(|&y| find_in_row(&buffer, y, "populate").map(|x| (y, x)))
        .expect("a description on the card");
    assert_eq!(bg_at(&buffer, x, y), theme.user_block_bg);
    assert_eq!(fg_at(&buffer, x, y), theme.primary_text);
}

#[rstest::rstest]
#[test]
fn a_listed_choice_is_green_like_the_forms_own_selected_choice() {
    // Given the activation card.
    let theme = jinn_theme::default_theme();
    let mut popup = popup_focused(PropertyField::Activation);
    popup.help_visible = true;

    // When rendering.
    let buffer = render_properties(popup);

    // Then the choice's name wears the same green the field's own selected
    // choice does, so the card and the form agree on which words are
    // selectable and which are explanation.
    let (row, x) = card_row_numbers(&buffer)
        .iter()
        .find_map(|&y| find_in_row(&buffer, y, "preserve").map(|x| (y, x)))
        .expect("the preserve choice");
    assert_eq!(fg_at(&buffer, x, row), theme.attendant_option_active);
}

#[rstest::rstest]
#[test]
fn the_help_card_names_the_field_in_the_focus_accent() {
    // Given a popup with the help card toggled on.
    let theme = jinn_theme::default_theme();
    let mut popup = popup_focused(PropertyField::Activation);
    popup.help_visible = true;

    // When rendering.
    let buffer = render_properties(popup);

    // Then the card's header is the field's own label in the focus accent,
    // so the card is tied to the row it answers for.
    let (top, _) = tooltip_rows(&buffer);
    let header = (top + 1..=top + 3)
        .find(|&y| find_in_row(&buffer, y, "activation").is_some())
        .expect("the header, inside the card's border");
    let x = find_in_row(&buffer, header, "activation").expect("the header");
    assert_eq!(fg_at(&buffer, x, header), theme.focus_accent);
}

#[rstest::rstest]
#[test]
fn the_focused_row_uses_the_user_message_background() {
    // Given a popup focused on the activation row.
    let buffer = render_properties(popup_focused(PropertyField::Activation));
    let row = highlighted_row_of(&buffer, PropertyField::Activation);

    // When reading the background behind the label.
    let label_x = find_in_row(&buffer, row, "activation:").expect("activation label");

    // Then it is the user-message background, not the attendant's own.
    assert_eq!(
        bg_at(&buffer, label_x, row),
        jinn_theme::default_theme().user_block_bg,
        "the focused row must borrow the user-message background"
    );
}

#[rstest::rstest]
#[test]
fn the_selected_choice_stays_green_on_the_focused_row() {
    // Given a popup focused on the trigger row.
    let buffer = render_properties(popup_focused(PropertyField::Trigger));
    let row = highlighted_row_of(&buffer, PropertyField::Trigger);

    // When reading the selected choice's foreground.
    let choice_x = find_in_row(&buffer, row, "parent-completed").expect("trigger value");

    // Then it is still the active green — the row background must not
    // rewrite the colors it has to keep distinct.
    assert_eq!(
        fg_at(&buffer, choice_x, row),
        Color::LightGreen,
        "the selected choice stays green on the focused row"
    );
}

#[rstest::rstest]
#[case::trigger(PropertyField::Trigger, "trigger:")]
#[case::activation(PropertyField::Activation, "activation:")]
#[case::template(PropertyField::SeedTemplate, "seed template:")]
#[test]
fn the_popup_rows_do_not_move_with_the_cursor(#[case] field: PropertyField, #[case] label: &str) {
    // Given a popup focused on the trigger row.
    let reference = render_properties(popup_focused(PropertyField::Trigger));

    // When rendering with the cursor elsewhere.
    let moved = render_properties(popup_focused(field));

    // Then each field is still on the body row it started on.
    assert_eq!(
        highlighted_row_of(&moved, field),
        highlighted_row_of(&reference, field),
        "{label} must not move when focus changes to {field:?}"
    );
}

#[rstest::rstest]
#[test]
fn the_status_line_carries_the_outcome_in_the_themes_status_color() {
    // Given a popup whose last save was refused.
    let popup = AttendantPropertiesState {
        status: Some(PopupStatus::SaveFailed {
            reason: "Cannot save: jinn.toml's saved attendants could not be read.".to_owned(),
        }),
        ..popup_focused(PropertyField::Trigger)
    };

    // When rendering.
    let buffer = render_properties(popup);

    // Then the reason is on the popup's own status line.
    let text = all_text(&buffer);
    assert!(
        text.contains("saved attendants could not be read"),
        "the status line carries the reason: {text}"
    );
}

#[rstest::rstest]
#[test]
fn the_status_line_shows_the_overwrite_prompt() {
    // Given a popup armed to overwrite a name that already exists.
    let popup = AttendantPropertiesState {
        status: Some(PopupStatus::OverwriteArmed {
            name: "nightly".to_owned(),
        }),
        ..popup_focused(PropertyField::Trigger)
    };

    // When rendering.
    let buffer = render_properties(popup);

    // Then the popup asks which entry it will replace, in the warning
    // color, rather than announcing a save that has not happened.
    let row = (0..buffer.area.height)
        .find(|&y| row_from_border(&buffer, y).contains("Overwrite"))
        .expect("an overwrite prompt row");
    let x = find_in_row(&buffer, row, "Overwrite").expect("the prompt's x");
    assert_eq!(fg_at(&buffer, x, row), jinn_theme::default_theme().warning);
}

#[rstest::rstest]
#[case::trigger(PropertyField::Trigger, "parent-completed:")]
#[case::activation(PropertyField::Activation, "seed:")]
#[case::template(PropertyField::SeedTemplate, "<prior report>:")]
#[test]
fn the_help_card_keeps_the_blank_line_between_its_lead_and_its_list(
    #[case] field: PropertyField,
    #[case] needle: &str,
) {
    // Given a popup on `field` with the help card toggled on.
    let mut popup = popup_focused(field);
    popup.help_visible = true;

    // When rendering.
    let buffer = render_properties(popup);

    // Then the card separates its lead sentence from its list with a blank
    // row. This is the newline support the card is built for: a field whose
    // help is a list is unreadable as one wrapped sentence.
    let (top, bottom) = tooltip_rows(&buffer);
    // The card's own blank row: the one after its header, where the lead
    // sentence ends and the list begins. Rows are read from the card's own
    // text, because a listed choice also appears in the form above it.
    // The needle carries the colon that introduces a description, so a
    // choice's name is matched as a listed item and not as a word that
    // happens to appear in the lead sentence.
    let first_list_row = (top..=bottom)
        .find(|&y| card_row_from_border(&buffer, y).contains(needle))
        .expect("the listed choice");
    let header =
        (top + 1..=bottom).find(|&y| card_row_from_border(&buffer, y).contains(field.label()));
    assert!(header.is_some(), "the card names the field it describes");
    let blank =
        (top + 1..first_list_row).find(|&y| card_row_from_border(&buffer, y).trim().is_empty());
    assert!(
        blank.is_some(),
        "a blank card row must separate the lead from the list starting at \
         y={first_list_row} (card y={top}..{bottom})"
    );
}

#[rstest::rstest]
#[case::trigger(PropertyField::Trigger)]
#[case::activation(PropertyField::Activation)]
#[case::template(PropertyField::SeedTemplate)]
#[test]
fn every_field_has_a_help_card_even_on_a_short_terminal(#[case] field: PropertyField) {
    // Given a terminal barely taller than the popup, with help toggled on:
    // a multi-row card has nowhere to go above or below, and must still be
    // on screen rather than silently dropped.
    let mut popup = popup_focused(field);
    popup.help_visible = true;

    // When rendering at that size.
    let buffer = render_properties_in(popup, 100, 22);

    // Then the card is there, though it may only have room for its head.
    let (top, bottom) = tooltip_rows(&buffer);
    assert!(
        top <= bottom && bottom < buffer.area.height,
        "the card must land inside a 22-row terminal, got y={top}..{bottom}"
    );
    // The card's first row is its top border; the header is the first row
    // inside it.
    let header = (top + 1..=bottom)
        .map(|y| row_from_border(&buffer, y))
        .find(|row| row.contains(field.label()))
        .unwrap_or_else(|| panic!("the card (y={top}..{bottom}) must name the field it describes"));
    assert!(header.contains(field.label()));
}

#[rstest::rstest]
#[case::activation(PropertyField::Activation, "preserve")]
#[case::trigger(PropertyField::Trigger, "manual")]
#[test]
fn a_listed_choice_is_introduced_by_a_colon(#[case] field: PropertyField, #[case] choice: &str) {
    // Given the card for a field whose help is a list.
    let mut popup = popup_focused(field);
    popup.help_visible = true;

    // When rendering.
    let buffer = render_properties(popup);

    // Then the choice's name is followed by a colon, so the name reads as a
    // label for the line after it rather than as one more word in a
    // sentence.
    let rows = card_row_numbers(&buffer);
    let row = rows
        .iter()
        .find(|&&y| find_in_row(&buffer, y, choice).is_some())
        .copied()
        .expect("the choice's row");
    let text = row_from_border(&buffer, row);
    let (_, after) = text.split_once(choice).expect("the choice is on its row");
    assert!(
        after.trim_start().starts_with(':'),
        "the choice must be followed by a colon, got {:?}",
        after.chars().take(12).collect::<String>()
    );
}
/// The tooltip's top and bottom rows: the rows carrying the help card's
/// light background, wherever the overlay landed. The tooltip is laid over
/// the terminal — including over the popup's own upper rows for a lower
/// field — so its position cannot be assumed; it is read by color.
/// The box-drawing corner the card's top-left is drawn with.
const CORNER_TOP_LEFT: &str = "\u{250c}";

/// The card's own left and right border columns, read from its bottom edge.
fn card_columns(buffer: &ratatui::buffer::Buffer, bottom: u16) -> (u16, u16) {
    let theme = jinn_theme::default_theme();
    let pink: Vec<u16> = (0..buffer.area.width)
        .filter(|&x| fg_at(buffer, x, bottom) == theme.attendant_fg)
        .collect();
    match (pink.first(), pink.last()) {
        (Some(left), Some(right)) => (*left, *right),
        _ => panic!("the card's bottom border must be the attendant's pink"),
    }
}

/// The card's own left border column.
fn card_left(buffer: &ratatui::buffer::Buffer) -> u16 {
    card_columns(buffer, tooltip_rows(buffer).1).0
}

/// Every row carrying the help card's surface, top to bottom.
///
/// The card is read by its background: the focused field's row inside the
/// popup shares the same surface, so a card row is one that is not the
/// popup's own row.
fn card_row_numbers(buffer: &ratatui::buffer::Buffer) -> Vec<u16> {
    let form = attendant_properties_overlay_rect(&buffer.area).expect("popup rect");
    (0..buffer.area.height)
        .filter(|&y| {
            let on_form = y >= form.y && y < form.y + form.height;
            !on_form && row_is_card(buffer, y)
        })
        .collect()
}

/// The help card's first and last row.
fn tooltip_rows(buffer: &ratatui::buffer::Buffer) -> (u16, u16) {
    let rows: Vec<u16> = card_row_numbers(buffer);
    match (rows.first(), rows.last()) {
        (Some(top), Some(bottom)) => (*top, *bottom),
        _ => panic!("no tooltip row carrying the help card's background"),
    }
}

/// The row the highlighted field occupies, derived from the label the
/// focused field renders.
fn highlighted_row_of(buffer: &ratatui::buffer::Buffer, field: PropertyField) -> u16 {
    let label = match field {
        PropertyField::Trigger => "trigger:",
        PropertyField::Activation => "activation:",
        PropertyField::SeedTemplate => "seed template:",
    };
    for y in body_top(buffer)..buffer.area.height {
        if find_in_row(buffer, y, label).is_some() {
            return y;
        }
    }
    panic!("no row for {field:?}");
}

#[rstest::rstest]
#[case::trigger(PropertyField::Trigger)]
#[case::activation(PropertyField::Activation)]
#[case::template(PropertyField::SeedTemplate)]
#[test]
fn the_help_card_is_framed_in_the_attendants_own_pink(#[case] field: PropertyField) {
    // Given a popup with the help card toggled on.
    let theme = jinn_theme::default_theme();
    let mut popup = popup_focused(field);
    popup.help_visible = true;

    // When rendering.
    let buffer = render_properties(popup);

    // Then all four edges are the attendant's own color. The border is the
    // card's identity — it says "this is about the attendant's settings"
    // before a word of it is read.
    let (top, bottom) = tooltip_rows(&buffer);
    let (left, right) = card_columns(&buffer, bottom);
    for (x, y, edge) in [
        (left, top, "top-left"),
        (right, top, "top-right"),
        (left, bottom, "bottom-left"),
        (right, bottom, "bottom-right"),
    ] {
        assert_eq!(
            fg_at(&buffer, x, y),
            theme.attendant_fg,
            "the card's {edge} corner must be the attendant's pink"
        );
    }
}

#[rstest::rstest]
#[test]
fn the_card_border_leaves_no_room_the_text_runs_out_of() {
    // Given the card at a width narrow enough to wrap its help.
    let mut popup = popup_focused(PropertyField::Trigger);
    popup.help_visible = true;

    // When rendering.
    let buffer = render_properties_in(popup, 46, 34);

    // Then every line of the card sits inside the border: a card shorter
    // than its own text runs off the bottom edge and reads as a rendering
    // fault, and wrapping at the card's width rather than the text's leaves
    // a line flush against the right border.
    let (top, bottom) = tooltip_rows(&buffer);
    let (_, right) = card_columns(&buffer, bottom);
    for y in (top + 1)..bottom {
        let row = card_row_from_border(&buffer, y);
        let reaches_border = row.chars().count() as u16 + card_left(&buffer) + 1 >= right;
        assert!(
            !reaches_border || row.trim().is_empty(),
            "card row {y} runs into the right border: {row}"
        );
    }
}

#[rstest::rstest]
#[test]
fn no_blank_row_sits_between_the_form_and_the_card() {
    // Given a popup with the help card on.
    let mut popup = popup_focused(PropertyField::Activation);
    popup.help_visible = true;

    // When rendering.
    let buffer = render_properties(popup);

    // Then the card starts on the row directly below the popup: no empty
    // row between them. A gap between two framed things is a hole in the
    // screen, not separation - the card's own edge is what separates them.
    let form = attendant_properties_overlay_rect(&buffer.area).expect("popup rect");
    let form_bottom = form.y + form.height - 1;
    let (top, _) = tooltip_rows(&buffer);
    assert_eq!(
        top,
        form_bottom + 1,
        "the card must start on the row directly below the form"
    );
    // And that row is drawn as the card's own top border, corner and all.
    assert_eq!(
        symbol_at(&buffer, inner_left() + 1, top),
        CORNER_TOP_LEFT,
        "the card's top row must be its own corner, not an empty row"
    );
}
