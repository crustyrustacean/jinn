//! The attendant properties popup — overlay geometry, view, and rows.
//!
//! The popup is a two-phase form. Its own scope is navigation-only: `j`/`k`
//! move the form cursor between the seven fields, `h`/`l` pick a choice
//! within the focused field, and `i` (on the seed-template field) opens the
//! template editor on its own capturing scope. Every edit stays pending in
//! the popup's cell until `<enter>` commits every field to the session
//! together; `<esc>`/`<c-c>` restore the open-time snapshot and close.
//!
//! Both popup phases render the full form — only the top scope's overlay is
//! drawn, so the editor view re-assembles the same rows and adds the text
//! cursor. The shared row assembly lives in [`properties_view`].

use super::properties_help_card::help_overlay;
use super::properties_permissions::pick_on;
use super::properties_save::{commit_pending_to_session, save_attendant};
use jinn_attendant_msg::{
    AttendantPropertiesState, PickDirection, PropertyField, attendant_seed_template_scope,
};
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{
    ActionCtx, ActionFn, BindSite, EditIntent, InputHook, RouteId, RouteOutcome, RouteRow,
    ScopeSignal,
};
use jinn_slices::{KeyRoutes, RenderFacts, RouteResult as IntentResult};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The typed cell the popup reads and writes.
pub(super) type AttendantPropertiesCell = TypedCell<AttendantPropertiesState>;

/// The form's fields in display order — the single list the view, the
/// popup's height, and the template row's offset are all derived from, so
/// adding a row cannot leave one of them behind.
///
/// `pub(crate)` so a test can walk the form by the same list the view draws,
/// rather than by a press count that a new row would silently falsify.
pub(crate) const FIELDS_IN_DISPLAY_ORDER: [PropertyField; 7] = [
    PropertyField::Trigger,
    PropertyField::Behavior,
    PropertyField::PrepMode,
    PropertyField::ToolSet,
    PropertyField::SkillSet,
    PropertyField::Model,
    PropertyField::SeedTemplate,
];

/// Horizontal padding fraction for the popup (20% each side).
const POPUP_H_PAD_FRAC: f32 = 0.20;
/// Minimum popup width in cells.
const POPUP_MIN_WIDTH: u16 = 44;
/// Popup content height: every field row, the status line, one footer line.
///
/// The help text is an overlay rather than a row in the form, so moving the
/// cursor no longer reflows the popup and this is a constant. It is derived
/// from the field list rather than written out, because a height that
/// disagrees with the rows the view draws is a popup that either clips a
/// field or wastes a row. The status line is a row: it holds a message, and a
/// message that reflowed the form would move the fields out from under the
/// cursor as it typed.
const POPUP_CONTENT_ROWS: u16 = FIELDS_IN_DISPLAY_ORDER.len() as u16 + 1 + 1;

/// Computes the centered properties popup rectangle: title, one row per
/// field, one hint line, and a keybind footer.
fn properties_popup_rect(area: Rect) -> Rect {
    let popup_width = ((f32::from(area.width) * (1.0 - 2.0 * POPUP_H_PAD_FRAC)).ceil() as u16)
        .max(POPUP_MIN_WIDTH)
        .min(area.width);
    let popup_height = (POPUP_CONTENT_ROWS + 2u16).min(area.height);

    #[expect(clippy::integer_division, reason = "cell positions are integers")]
    let popup_x = area.width.saturating_sub(popup_width) / 2;
    #[expect(clippy::integer_division, reason = "cell positions are integers")]
    let popup_y = area.height.saturating_sub(popup_height) / 3;

    Rect::new(popup_x, popup_y, popup_width, popup_height)
}

/// The overlay-rect function registered on the slice host.
pub fn attendant_properties_overlay_rect(area: &Rect) -> Option<Rect> {
    Some(properties_popup_rect(*area))
}

/// The overlay view for the properties scope: the full form, no cursor.
///
/// The overlay only renders when the slice that owns the cell activated
/// (the `read_popup` bootstrap assertion covers the missing-cell case).
pub fn render_attendant_properties(frame: &mut Frame<'_>, area: Rect, ctx: &RenderFacts) {
    let popup = read_popup(ctx);
    let theme = &ctx.theme;

    frame.render_widget(Clear, area);
    render_popup_frame(frame, area, theme);
    let inner = inner_rect(area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    // The popup's geometry and draft window are shared by the two phases,
    // so the form and the cursor can never disagree about what is visible.
    let layout = properties_layout(&popup, inner);
    frame.render_widget(
        Paragraph::new(properties_view(&popup, theme, &layout)),
        inner,
    );
    // The help overlay is anchored to the highlighted row, not to a
    // terminal cursor: this popup is navigation-only and owns no cursor of
    // its own, so its position follows the row the user is reading.
    if let Some(help) = help_overlay(&popup, area, inner, frame.area(), theme) {
        help.draw(frame);
    }
}

/// The overlay view for the seed-template editor scope: the same form plus
/// the text cursor on the template row.
///
/// Only the top scope's overlay draws, so the editor view re-assembles the
/// properties rows. A missing overlay registration would blank the popup
/// while editing — the boot probe asserts the registration exists.
///
/// The editor only opens from the properties popup, which only renders
/// when the slice that owns the cell activated (the `read_popup` bootstrap
/// assertion covers the missing-cell case).
pub fn render_attendant_seed_template(frame: &mut Frame<'_>, area: Rect, ctx: &RenderFacts) {
    let popup = read_popup(ctx);
    let theme = &ctx.theme;

    frame.render_widget(Clear, area);
    render_popup_frame(frame, area, theme);
    let inner = inner_rect(area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    // The popup's geometry and draft window are shared by the two phases,
    // so the form and the cursor can never disagree about what is visible.
    let layout = properties_layout(&popup, inner);
    frame.render_widget(
        Paragraph::new(properties_view(&popup, theme, &layout)),
        inner,
    );
    render_template_cursor(frame, inner, &layout);
}

/// Reads the popup's cell from the render facts.
///
/// # Panics
///
/// Panics if the slot is not registered — both overlays render only when
/// the attendant slice activated.
#[expect(
    clippy::expect_used,
    reason = "both overlays render only when the attendant slice registered the cell"
)]
fn read_popup(ctx: &RenderFacts) -> AttendantPropertiesState {
    ctx.slices
        .reader::<AttendantPropertiesState>(&jinn_attendant_msg::attendant_properties_slot())
        .expect("properties overlay renders only when the attendant cell is registered")
        .read()
        .clone()
}

/// Draws the popup's border and title. Shared by both phases.
fn render_popup_frame(frame: &mut Frame<'_>, area: Rect, theme: &jinn_theme::Theme) {
    let block = Block::default()
        .title(Span::styled(
            " Attendant Properties ",
            Style::default().fg(theme.popup_title),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_unfocused));
    frame.render_widget(block, area);
}

/// The popup body rect inside its border.
fn inner_rect(area: Rect) -> Rect {
    Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

/// The marker span: `▸ ` when focused, `  ` otherwise.
fn marker_text(focused: bool) -> &'static str {
    if focused { "▸ " } else { "  " }
}

/// The field label span: the label, then the `:  ` that closes it.
fn field_label(field: PropertyField) -> String {
    format!("{}:  ", field.label())
}

/// The column the seed-template value starts at, in screen cells.
///
/// Measured from the very spans [`field_line`] puts before it — the marker
/// and the label — so the window, the text, and the cursor cannot disagree.
/// It must be a *cell* count, not a character count: `▸` is an East Asian
/// Wide glyph occupying two cells, so the focused row's value starts one
/// cell later than the unfocused row's.
fn value_column(field: PropertyField, focused: bool) -> u16 {
    let marker = u16::try_from(UnicodeWidthStr::width(marker_text(focused))).unwrap_or(2);
    let label =
        u16::try_from(UnicodeWidthStr::width(field_label(field).as_str())).unwrap_or(u16::MAX);
    marker.saturating_add(label)
}

/// Where the seed-template draft sits in the popup, and which slice of it
/// is visible.
///
/// A draft longer than the row cannot be drawn in full, so the row shows a
/// **window** of it. The window starts at the first grapheme that still
/// fits, advancing as the cursor moves right so the cursor is always on
/// screen; when it moves back to the start the window follows it. The
/// rendered text and the cursor column both come from here, so they cannot
/// disagree.
#[derive(Debug, Clone, Copy)]
struct PropertiesLayout {
    /// The template draft's row, in screen coordinates.
    template_y: u16,
    /// How many draft graphemes the row can show.
    window_width: u16,
    /// The draft grapheme index the window starts at.
    window_start: usize,
    /// The draft's value column: the first cell of the window, in screen
    /// coordinates.
    value_x: u16,
    /// The cursor's column within the row, measured from the window's start
    /// (the draft is left-aligned in its window, so the two are equal).
    cursor_offset: u16,
}

/// Computes the draft window and cursor column for `popup` inside `inner`.
///
/// The draft's row is a fixed offset below the popup's first body row: the
/// form renders its fields in display order, with no hint row of its own,
/// so the template is always the last field and the fields above it are
/// always there. Deriving the count from the field list rather than
/// hardcoding it is what keeps a fifth row from silently rendering the
/// cursor one row above the draft.
fn properties_layout(popup: &AttendantPropertiesState, inner: Rect) -> PropertiesLayout {
    // Every field above the template, counted from the same list the view
    // renders.
    let rows_above = u16::try_from(FIELDS_IN_DISPLAY_ORDER.len() - 1).unwrap_or(u16::MAX);
    // The value starts at the label's end, and one cell is reserved so the
    // cursor can rest just past the last visible grapheme. The focused
    // marker is wide, so the window is one narrower than an unfocused row's.
    let focused = popup.focus == PropertyField::SeedTemplate;
    let value_col = value_column(PropertyField::SeedTemplate, focused);
    let window_width = inner.width.saturating_sub(value_col + 1);
    let draft = &popup.seed_template;
    let total = grapheme_count(&draft.input);
    let cursor_index = draft
        .input
        .get(..draft.cursor_pos)
        .map_or(total, |before| before.graphemes(true).count());
    let window_start = window_start(total, cursor_index, window_width);
    PropertiesLayout {
        template_y: inner.y.saturating_add(rows_above),
        window_width,
        window_start,
        value_x: inner.x.saturating_add(value_col),
        cursor_offset: cursor_index.saturating_sub(window_start) as u16,
    }
}

/// The window start that keeps `cursor_index` visible in a
/// `window_width`-grapheme row: the last full window that ends at or after
/// the cursor, and zero when the whole draft fits.
fn window_start(total: usize, cursor_index: usize, window_width: u16) -> usize {
    let width = usize::from(window_width);
    if width == 0 || total <= width {
        return 0;
    }
    total.saturating_sub(width).min(cursor_index)
}

/// The number of grapheme clusters in `text`; `0` for a byte range that is
/// not a valid char boundary.
fn grapheme_count(text: &str) -> usize {
    text.graphemes(true).count()
}

/// Places the terminal cursor inside the visible template draft.
fn render_template_cursor(frame: &mut Frame<'_>, inner: Rect, layout: &PropertiesLayout) {
    let cursor_x = layout
        .value_x
        .saturating_add(layout.cursor_offset)
        .min(inner.x.saturating_add(inner.width).saturating_sub(1));
    if layout.template_y < inner.y + inner.height {
        frame.set_cursor_position((cursor_x, layout.template_y));
    }
}

/// Assembles the popup body from the popup state and theme.
///
/// One line per field: marker + label (yellow iff focused), then the
/// field's value spans. A hint line renders only under the focused field,
/// and the footer names the keys that work on the focused field. The
/// template draft shows the window [`properties_layout`] computed, so the
/// visible text and the cursor always agree.
fn properties_view<'a>(
    popup: &'a AttendantPropertiesState,
    theme: &'a jinn_theme::Theme,
    layout: &PropertiesLayout,
) -> Vec<Line<'a>> {
    let mut lines = vec![];
    for field in FIELDS_IN_DISPLAY_ORDER {
        lines.push(field_line(popup, field, theme, layout));
    }
    lines.push(status_line(popup, theme));
    lines.push(footer_line(popup.focus, theme));
    lines
}

/// The status line: what the last key did, in the tone that fits.
///
/// The line is always rendered, empty when there is nothing to say, so the
/// popup's fields never move up or down as messages come and go.
pub(crate) fn status_line(
    popup: &AttendantPropertiesState,
    theme: &jinn_theme::Theme,
) -> Line<'static> {
    let (text, color) = match &popup.status {
        None => (String::new(), theme.muted_text),
        Some(jinn_attendant_msg::PopupStatus::OverwriteArmed { name }) => (
            format!("Overwrite “{name}”? Press ctrl-s again to replace it."),
            theme.warning,
        ),
        Some(jinn_attendant_msg::PopupStatus::Saved { name }) => {
            (format!("Saved “{name}”."), theme.success)
        }
        Some(jinn_attendant_msg::PopupStatus::SaveFailed { reason }) => {
            (reason.clone(), theme.error_text)
        }
        Some(jinn_attendant_msg::PopupStatus::GlobDropped { field }) => (
            format!(
                "A manually-configured glob was dropped from the {} set.",
                field.resource()
            ),
            theme.warning,
        ),
    };
    Line::from(Span::styled(text, Style::default().fg(color)))
}

/// One hint as (key, description) — the keys the focused field responds to.
fn hints(focus: PropertyField) -> Vec<(&'static str, &'static str)> {
    let mut hints = match focus {
        PropertyField::SeedTemplate => vec![("i", "edit"), ("j/k", "field")],
        _ => vec![("h/l", "pick"), ("j/k", "field")],
    };
    hints.push(("?", "help"));
    // The save hint is only true while the name is savable, and the popup
    // is what tells the user an attendant has no name yet — so the key is
    // advertised on the form and the status line explains the one case
    // where pressing it will not save.
    hints.push(("<c-s>", "save"));
    hints
}

/// The footer: the keys that work on the focused field.
///
/// Key glyphs carry the theme's `accent_action` — the hotkey accent every
/// other panel's hints use — and the descriptions stay muted, so the key a
/// user is about to press is the part that stands out.
fn footer_line(focus: PropertyField, theme: &jinn_theme::Theme) -> Line<'static> {
    let key_style = Style::default().fg(theme.accent_action);
    let text_style = Style::default().fg(theme.muted_text);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let entries = hints(focus)
        .into_iter()
        .chain([("<enter>", "apply"), ("<esc>", "cancel")])
        .collect::<Vec<_>>();
    for (index, (key, label)) in entries.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" · ".to_owned(), text_style));
        }
        spans.push(Span::styled(key.to_owned(), key_style));
        spans.push(Span::styled(format!(" {label}"), text_style));
    }
    Line::from(spans)
}

/// One field row: focused marker + yellow label, then the value spans.
///
/// A row that does not apply while the attendant is being composed is
/// dimmed: its label and its unselected choices go to the muted color, so
/// the form says on its face which settings are inert. The *selected*
/// choice keeps its green — the value the user wrote is still theirs, and
/// muting it would read as "unset" rather than "written but not in effect".
fn field_line<'a>(
    popup: &AttendantPropertiesState,
    field: PropertyField,
    theme: &'a jinn_theme::Theme,
    layout: &PropertiesLayout,
) -> Line<'a> {
    let focused = popup.focus == field;
    let dim = popup.pending_prep_mode && !field.applies_while_prepping();
    let mut spans = vec![
        field_marker(focused, dim, theme),
        field_name(field, focused, dim, theme),
    ];
    match field {
        PropertyField::Trigger => spans.extend(choice_spans(
            jinn_attendant_msg::TRIGGER_CHOICES,
            &popup.pending_trigger,
            dim,
            theme,
        )),
        PropertyField::Behavior => spans.extend(choice_spans(
            jinn_attendant_msg::BEHAVIOR_CHOICES,
            &popup.pending_behavior,
            dim,
            theme,
        )),
        PropertyField::PrepMode => spans.extend(prep_mode_spans(popup.pending_prep_mode, theme)),
        PropertyField::ToolSet => spans.extend(choice_spans(
            jinn_attendant_msg::SET_MODE_CHOICES,
            &popup.pending_tool_set,
            dim,
            theme,
        )),
        PropertyField::SkillSet => spans.extend(choice_spans(
            jinn_attendant_msg::SET_MODE_CHOICES,
            &popup.pending_skill_set,
            dim,
            theme,
        )),
        PropertyField::Model => spans.extend(choice_spans(
            jinn_attendant_msg::MODEL_CHOICES,
            &popup.pending_model_setting,
            dim,
            theme,
        )),
        PropertyField::SeedTemplate => {
            spans.push(template_value(popup, theme, layout));
        }
    }
    // The focused row carries the user-message background across its whole
    // width, so the cursor reads as a selected row rather than a tinted
    // label. Every span keeps the foreground it would have had unfocused —
    // the selected choice stays green beside plain text, and a background
    // is not a licence to rewrite the colors that distinction depends on.
    if focused {
        return Line::from(spans).style(focused_row_style(theme));
    }
    Line::from(spans)
}

/// The prep row's value: what composition means, stated on the row.
///
/// `[on]` is a plain-text state with a warning beside it — not a selected
/// choice, because nothing is being *chosen* between two peers; the
/// alternative, composing, is what the state means. `[off]` wears the
/// selected-choice green, because that is the state a user reaching for the
/// attendant runs in, and it is the one the eye should find.
fn prep_mode_spans(prep_mode: bool, theme: &jinn_theme::Theme) -> Vec<Span<'_>> {
    if prep_mode {
        vec![
            Span::styled(
                PREP_MODE_ON.to_owned(),
                Style::default().fg(theme.primary_text),
            ),
            Span::styled(
                PREP_MODE_DISABLED_NOTE.to_owned(),
                Style::default().fg(theme.error_text),
            ),
        ]
    } else {
        vec![Span::styled(
            PREP_MODE_OFF.to_owned(),
            Style::default().fg(theme.attendant_option_active),
        )]
    }
}

/// The prep row's on-state label, and what it costs.
const PREP_MODE_ON: &str = "[on]";
const PREP_MODE_OFF: &str = "[off]";
const PREP_MODE_DISABLED_NOTE: &str = "  (Attendant disabled)";

/// The focused row's background: the user-message block, so the row reads
/// as a selection against a surface the user already knows rather than as a
/// new color introduced by this popup.
fn focused_row_style(theme: &jinn_theme::Theme) -> Style {
    Style::default().bg(theme.user_block_bg)
}

/// The focused-field marker: `▸` when focused, blank otherwise.
///
/// A dimmed row's marker goes muted with its label. The cage makes the
/// cursor unable to sit here, so a marker in the focus accent on a row the
/// user cannot reach would be a claim the popup cannot keep.
fn field_marker(focused: bool, dim: bool, theme: &jinn_theme::Theme) -> Span<'static> {
    Span::styled(
        marker_text(focused).to_owned(),
        Style::default().fg(row_foreground(focused, dim, theme)),
    )
}

/// The field's label, yellow only while its row is focused, muted when the
/// row does not currently apply.
fn field_name(
    field: PropertyField,
    focused: bool,
    dim: bool,
    theme: &jinn_theme::Theme,
) -> Span<'static> {
    Span::styled(
        field_label(field),
        Style::default().fg(row_foreground(focused, dim, theme)),
    )
}

/// A row's label color: the focus accent on the focused row, the muted
/// color on a row that does not apply, plain text otherwise.
fn row_foreground(focused: bool, dim: bool, theme: &jinn_theme::Theme) -> ratatui::style::Color {
    if focused {
        theme.focus_accent
    } else if dim {
        theme.muted_text
    } else {
        theme.primary_text
    }
}

/// The choice spans for a choice row: the selected choice in the active
/// green, the rest in plain text, separated by muted slashes.
///
/// `dim` mutes the *unselected* choices only. The selected one stays green
/// on a dimmed row so the value reads as written-and-held rather than
/// unset, and so that turning prep mode off shows the same value, in the
/// same color, the user last chose.
fn choice_spans<'a, T>(
    choices: &[(T, &'static str)],
    selected: &T,
    dim: bool,
    theme: &'a jinn_theme::Theme,
) -> Vec<Span<'a>>
where
    T: PartialEq,
{
    let mut spans = vec![];
    for (index, (value, label)) in choices.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" / ", Style::default().fg(theme.muted_text)));
        }
        let style = if *value == *selected {
            Style::default().fg(theme.attendant_option_active)
        } else if dim {
            Style::default().fg(theme.muted_text)
        } else {
            Style::default().fg(theme.primary_text)
        };
        spans.push(Span::styled((*label).to_owned(), style));
    }
    spans
}

/// The template text span: the live draft's visible window, in plain text
/// color, cut by grapheme (never by byte or char) at both edges so a long
/// draft scrolls with the cursor instead of showing only its head.
fn template_value<'a>(
    popup: &AttendantPropertiesState,
    theme: &'a jinn_theme::Theme,
    layout: &PropertiesLayout,
) -> Span<'a> {
    let window: String = popup
        .seed_template
        .input
        .graphemes(true)
        .skip(layout.window_start)
        .take(usize::from(layout.window_width))
        .collect();
    // The window is sized in *columns*; wide graphemes (CJK, emoji) take
    // two, so clip the rendered text to the window's column budget.
    Span::styled(
        clip_to_columns(&window, layout.window_width),
        Style::default().fg(theme.primary_text),
    )
}

/// The screen width of one grapheme cluster, in columns: two for East
/// Asian Wide and Fullwidth characters, one otherwise.
fn grapheme_width(grapheme: &str) -> u16 {
    u16::try_from(UnicodeWidthStr::width(grapheme)).unwrap_or(u16::MAX)
}

/// `text` cut to at most `max_columns` screen columns, never splitting a
/// grapheme cluster.
fn clip_to_columns(text: &str, max_columns: u16) -> String {
    let mut used: u16 = 0;
    let mut out = String::new();
    for grapheme in text.graphemes(true) {
        let width = grapheme_width(grapheme);
        if used.saturating_add(width) > max_columns {
            break;
        }
        used += width;
        out.push_str(grapheme);
    }
    out
}

/// Builds the popup's input hook: typed keys edit the seed template draft.
///
/// Registered against the seed-template editor's scope (the properties
/// scope captures nothing) — or typed characters would reach nothing.
#[must_use]
pub fn attendant_properties_input_hook(cell: &AttendantPropertiesCell) -> InputHook {
    let cell = cell.clone();
    std::sync::Arc::new(move |intent: &EditIntent| {
        let cell = cell.clone();
        // Typing is a keystroke like any other; the status line describes
        // the last one, and an armed-overwrite message must not outlive
        // the keystroke that was supposed to follow it.
        cell.update(AttendantPropertiesState::clear_status);
        match intent {
            EditIntent::InsertChar(ch) => {
                cell.update(|s| s.seed_template.insert_char(*ch));
                Some(IntentResult::empty())
            }
            EditIntent::DeleteBackward => {
                cell.update(|s| s.seed_template.delete());
                Some(IntentResult::empty())
            }
            EditIntent::DeleteForward => {
                cell.update(|s| s.seed_template.delete_forward());
                Some(IntentResult::empty())
            }
            EditIntent::Paste(text) => {
                // The seed-template field is a single-line `LineInput`, so
                // its paste flattens line breaks like every other one.
                let text = text.clone();
                cell.update(move |s| s.seed_template.paste(&text));
                Some(IntentResult::empty())
            }
            EditIntent::CursorLeft => {
                cell.update(|s| s.seed_template.cursor_left());
                Some(IntentResult::empty())
            }
            EditIntent::CursorRight => {
                cell.update(|s| s.seed_template.cursor_right());
                Some(IntentResult::empty())
            }
            EditIntent::CursorHome => {
                cell.update(|s| s.seed_template.cursor_home());
                Some(IntentResult::empty())
            }
            EditIntent::CursorEnd => {
                cell.update(|s| s.seed_template.cursor_end());
                Some(IntentResult::empty())
            }
        }
    })
}

/// The kernel's application state behind an [`ActionCtx`].
pub(super) fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> Option<&'a mut jinn_kernel::AppState> {
    ctx.state
        .as_any_mut()?
        .downcast_mut::<jinn_kernel::AppState>()
}

/// Wraps a popup action in an [`ActionFn`], handing it the cell.
///
/// Every row's action starts by clearing the status line, so the line
/// always describes the most recent keystroke and nothing else. Doing it
/// here rather than in each action is what makes that true: a row added
/// later inherits the rule instead of having to remember it, and an action
/// that reports — the save — writes its message after the clear and keeps
/// it.
pub(super) fn action<F>(cell: &AttendantPropertiesCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &AttendantPropertiesCell) -> IntentResult + Send + Sync + 'static,
{
    let cell = cell.clone();
    ActionFn::new(move |mut ctx| {
        cell.update(AttendantPropertiesState::clear_status);
        f(&mut ctx, &cell)
    })
}

/// Like [`action`], but the wrapped function runs before the status line
/// is cleared.
///
/// Only the save needs this. Every other keystroke withdraws an armed
/// overwrite — see [`AttendantPropertiesState::clear_status`] — but the
/// save is the keystroke the arming was *for*: clearing first would disarm
/// the confirmation press and the second `<c-s>` would silently start the
/// whole two-press dance again. The save is also the one action whose
/// outcome it writes itself, so the clear that follows takes away nothing
/// it needs.
fn action_preserving_arm<F>(cell: &AttendantPropertiesCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &AttendantPropertiesCell) -> IntentResult + Send + Sync + 'static,
{
    let cell = cell.clone();
    ActionFn::new(move |mut ctx| f(&mut ctx, &cell))
}

/// Builds an own-scope row for one of the popup's scopes.
pub(super) fn row(
    route_id: &'static str,
    scope: jinn_slices::SliceScopeId,
    key: &'static str,
    category: &'static str,
    display: &'static str,
    run: ActionFn,
) -> RouteRow {
    RouteRow {
        route_id: RouteId::new(route_id),
        scope,
        key,
        category,
        site: BindSite::OwnScope,
        feature: "attendant",
        outcome: RouteOutcome::Action {
            action: route_id,
            display,
            run,
        },
    }
}

/// Attaches every row the properties form owns.
///
/// The `P` opener lives with the sidebar's sessions rows, because `P` means
/// "edit the highlighted session's attendant" there. The editor's keep/
/// restore/clear rows land on the editor scope in
/// [`attach_seed_template_rows`]; this function owns the form itself.
pub fn attach_properties_rows(routes: &KeyRoutes, cell: &AttendantPropertiesCell) {
    let properties_scope = jinn_attendant_msg::attendant_properties_scope();

    routes.attach(row(
        "attendant-properties-field-next",
        properties_scope.clone(),
        "j",
        "navigation",
        "next field",
        action(cell, |_, cell| {
            cell.update(AttendantPropertiesState::focus_next);
            IntentResult::empty()
        }),
    ));
    routes.attach(row(
        "attendant-properties-field-previous",
        properties_scope.clone(),
        "k",
        "navigation",
        "previous field",
        action(cell, |_, cell| {
            cell.update(AttendantPropertiesState::focus_previous);
            IntentResult::empty()
        }),
    ));
    routes.attach(row(
        "attendant-properties-help",
        properties_scope.clone(),
        "?",
        "general",
        "show the help overlay",
        action(cell, |_, cell| {
            cell.update(|popup| popup.help_visible = !popup.help_visible);
            IntentResult::empty()
        }),
    ));
    routes.attach(row(
        "attendant-properties-pick-left",
        properties_scope.clone(),
        "h",
        "navigation",
        "pick the previous choice",
        action(cell, |ctx, cell| pick_on(ctx, cell, PickDirection::Left)),
    ));
    routes.attach(row(
        "attendant-properties-pick-right",
        properties_scope.clone(),
        "l",
        "navigation",
        "pick the next choice",
        action(cell, |ctx, cell| pick_on(ctx, cell, PickDirection::Right)),
    ));
    routes.attach(row(
        "attendant-properties-edit-template",
        properties_scope.clone(),
        "i",
        "general",
        "edit the seed template",
        action(cell, |_ctx, cell| {
            // `i` means "edit the template" only from the template field.
            if cell.read().focus != PropertyField::SeedTemplate {
                return IntentResult::empty();
            }
            cell.update(AttendantPropertiesState::begin_template_edit);
            IntentResult::empty()
                .with_scope_signal(ScopeSignal::Push(attendant_seed_template_scope()))
        }),
    ));
    routes.attach(row(
        "attendant-properties-save",
        properties_scope.clone(),
        "<c-s>",
        "general",
        "save this attendant to jinn.toml",
        action_preserving_arm(cell, save_attendant),
    ));
    routes.attach(row(
        "attendant-properties-apply",
        properties_scope.clone(),
        "<enter>",
        "general",
        "apply all fields and close",
        action(cell, apply_all_fields),
    ));
    for (route_id, key) in [
        ("attendant-properties-leave", "<esc>"),
        ("attendant-properties-cancel", "<c-c>"),
    ] {
        routes.attach(row(
            route_id,
            properties_scope.clone(),
            key,
            "general",
            "restore originals and close",
            action(cell, |_, cell| {
                cell.update(AttendantPropertiesState::restore_original);
                IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(
                    jinn_attendant_msg::attendant_properties_scope(),
                ))
            }),
        ));
    }
}

/// The apply row's action: commits the popup's pending values to the
/// session it names, together, and persists once.
fn apply_all_fields(ctx: &mut ActionCtx<'_>, cell: &AttendantPropertiesCell) -> IntentResult {
    let popup = cell.read().clone();
    let Some(attendant_id) = popup.session_id.clone() else {
        return IntentResult::empty();
    };
    commit_pending_to_session(ctx, &popup, &attendant_id);
    // Leaving the popup disarms the save, exactly as `<esc>` does: an arm
    // confirmed against a name the user has since changed must not survive
    // into the next popup session.
    cell.update(AttendantPropertiesState::disarm_save);
    IntentResult::empty()
        .with_message(jinn_session_store_msg::PersistSession {
            session_id: attendant_id,
        })
        .with_scope_signal(ScopeSignal::PopIf(
            jinn_attendant_msg::attendant_properties_scope(),
        ))
}
