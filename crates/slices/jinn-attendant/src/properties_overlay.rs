//! The attendant properties popup — overlay geometry, view, and rows.
//!
//! The popup is a two-phase form. Its own scope is navigation-only: `j`/`k`
//! move the form cursor between the six fields, `h`/`l` pick a choice
//! within the focused field, and `i` (on the seed-template field) opens the
//! template editor on its own capturing scope. Every edit stays pending in
//! the popup's cell until `<enter>` commits every field to the session
//! together; `<esc>`/`<c-c>` restore the open-time snapshot and close.
//!
//! Both popup phases render the full form — only the top scope's overlay is
//! drawn, so the editor view re-assembles the same rows and adds the text
//! cursor. The shared row assembly lives in [`properties_view`].

use std::collections::BTreeSet;

use jinn_attendant_msg::{
    AttendantPropertiesState, PickDirection, PopupStatus, PropertyField, SetField, SetMode,
    attendant_seed_template_scope,
};
use jinn_core_types::{FilterMode, NameFilter};
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
type AttendantPropertiesCell = TypedCell<AttendantPropertiesState>;

/// The form's fields in display order — the single list the view, the
/// popup's height, and the template row's offset are all derived from, so
/// adding a row cannot leave one of them behind.
const FIELDS_IN_DISPLAY_ORDER: [PropertyField; 6] = [
    PropertyField::Trigger,
    PropertyField::Behavior,
    PropertyField::PrepMode,
    PropertyField::ToolSet,
    PropertyField::SkillSet,
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
        frame.render_widget(Clear, help.area);
        // The frame first — it draws the border and paints the card's
        // surface — then the text into the area the frame leaves.
        frame.render_widget(help.card, help.area);
        frame.render_widget(Paragraph::new(help.lines), help.text);
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

/// The hint line under the focused field.
/// The help card's body, for the focused field.
///
/// The body is a [`Line`], not a string: a field's help is a list of
/// choices or a template, and a list reads as a list. Wrapping one choice
/// per row also puts each name where a reader expects it, instead of
/// leaving them mid-sentence on whatever row the wrap happened to break.
///
/// The text is plain on purpose. The card is a [`Paragraph`] and its
/// construction is the place to add emphasis — see [`help_paragraph`].
fn help_body(field: PropertyField, theme: &jinn_theme::Theme) -> Vec<Line<'static>> {
    // A choice's name is the same green the field's own selected choice
    // wears, so the card and the form agree on which words are selectable
    // and which are explanation.
    let choice = Style::default().fg(theme.attendant_option_active);
    let said = Style::default().fg(theme.primary_text);
    let line = |name: &str, description: &str| {
        Line::from(vec![
            Span::raw("  "),
            Span::styled(name.to_owned(), choice),
            Span::styled(format!(":  {description}"), said),
        ])
    };
    match field {
        PropertyField::Trigger => vec![
            Line::from("Condition to trigger the attendant."),
            Line::from(""),
            line("parent-completed", "runs when the agent finishes it's turn"),
            line("manual", "only the `R` keybind will run the attendant"),
        ],
        PropertyField::Behavior => vec![
            Line::from("How to manage the context when the attendant is triggered."),
            Line::from(""),
            line("reset", "pins are kept, all other context excluded/hidden"),
            line("preserve", "one continuous chat session; no context edits"),
        ],
        PropertyField::PrepMode => vec![
            Line::from(
                "Attendants need to be 'prepped' before usage by providing pinned context to define their behavior. Messages land in context pinned and not sent to a provider.",
            ),
            Line::from(""),
            line(
                "on",
                "prepare the session by submitting messages; attendant disabled",
            ),
            line("off", "attendant is active"),
        ],
        PropertyField::ToolSet => vec![
            Line::from(
                "Controls the working tool set and how tool enablement works. If you manually configured globs in the config file, then they will be dropped upon switching modes (their meaning will invert)",
            ),
            Line::from(""),
            line(
                "live",
                "(block list) - Tools that were disabled for the attendant stay disabled. New tools are automatically enabled, mirroring jinn's enabled-by-default behavior",
            ),
            line(
                "frozen",
                "(allow list) - The tools enabled for the attendant are the only ones available. New tools are automatically disabled.",
            ),
        ],
        PropertyField::SkillSet => vec![
            Line::from(
                "Controls the working skill set and how skill enablement works. If you manually configured globs in the config file, then they will be dropped upon switching modes (their meaning will invert).",
            ),
            Line::from(""),
            line(
                "live",
                "(block list) - Skills that were disabled for the attendant stay disabled. New skills are automatically enabled, mirroring jinn's enabled-by-default behavior",
            ),
            line(
                "frozen",
                "(allow list) - The skills enabled for the attendant are the only ones available. New skills are automatically disabled.",
            ),
        ],
        PropertyField::SeedTemplate => vec![
            Line::from(
                "Text injected on each run, ahead of the previous report. Useful when using `reset` behavior to advise the agent of the previous conclusion.",
            ),
            Line::from(""),
            line(
                "<prior report>",
                "replacement token you can optionally add to the see template",
            ),
        ],
    }
}

/// The cells the help card's border takes off its own width: one on each
/// side. The text is drawn inside them.
const HELP_CARD_BORDER_CELLS: u16 = 2;

/// The help card's text: a header naming the field, then its body lines.
///
/// Every row carries the card's own background, and its foreground is
/// chosen by the row's *role* — header, body — so adding emphasis means
/// giving a line a color, not restating the surface it sits on. The body
/// lines are wrapped here rather than handed to the `Paragraph`, because the
/// card sizes itself to its content and must know its height before it can
/// be placed.
fn help_body_lines(
    field: PropertyField,
    inner_width: u16,
    theme: &jinn_theme::Theme,
) -> Vec<Line<'static>> {
    let text_width = inner_width.max(1);
    let mut lines = vec![Line::from(Span::styled(
        // The label as the form shows it, minus the colon padding: the
        // card is a description of the field, not a copy of its row.
        field.label().to_owned(),
        Style::default().fg(theme.focus_accent),
    ))];
    lines.extend(
        help_body(field, theme)
            .into_iter()
            .flat_map(|line| wrap_line(line, text_width))
            // `Line::style` replaces rather than patches, so the surface is
            // restated on every row: a body's own colors layered on a card
            // background would keep the foreground and drop the surface.
            .map(|line| line.style(Style::default().bg(theme.user_block_bg))),
    );
    // A blank line in the authored body is the author's own spacing and is
    // kept as authored; the card adds no pad of its own, so its height is
    // exactly its text.
    lines
}

/// The help card's frame: the attendant's own pink around its text.
///
/// The border is the card's identity — it says *this box is about the
/// attendant's settings* the moment it appears, before a word of it is
/// read, and it keeps the card from bleeding into whatever the chat log
/// happened to have behind it. It is [`attendant_fg`] rather than a new
/// color because that pink is already the app's word for "attendant".
///
/// The block carries the card's surface as well as its edge, for the same
/// reason the rows do: a `Paragraph` paints only the cells a line's text
/// covers, and a card with a blank row in it would otherwise show a hole
/// in its own background rather than a blank row of card.
fn help_card(theme: &jinn_theme::Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(theme.attendant_fg)
                .bg(theme.user_block_bg),
        )
        .style(Style::default().bg(theme.user_block_bg))
}

/// Wraps one help line to `width`, breaking on whitespace, and returns the
/// rows it occupies.
///
/// A word too long for a row of its own is placed on the next row rather
/// than split mid-word; that is the only case that can overflow, and it is
/// better one long row than a word broken across two. An empty line stays
/// exactly one row: it is the blank the authored help asked for.
fn wrap_line(line: Line<'static>, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    if line.spans.iter().all(|span| span.content.is_empty()) {
        // A line of nothing: it asked for a row, and wrapping it to no rows
        // at all would drop the author's blank.
        return vec![line];
    }
    // Words are separated by their *authored* gap, and a run of whitespace is
    // a run: the help lines are aligned columns, and collapsing the gap
    // between a name and its description to one space would throw away the
    // only thing that makes the list a list.
    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    for span in line.spans {
        // The whitespace between the span's words, as authored, and a row
        // edge: an authored gap cannot be measured before the words around
        // it are known, so it is buffered here rather than emitted.
        let mut gap = String::new();
        let mut at_row_start = true;
        for token in span.content.split(' ') {
            if token.is_empty() {
                // A run of spaces: the run's own length is the gap, and it
                // collapses to the count that fits.
                if !at_row_start {
                    gap.push(' ');
                }
                continue;
            }
            let length = token.chars().count();
            let separator = usize::min(gap.len(), width.saturating_sub(1));
            if used + separator + length > width {
                rows.push(Line::from(std::mem::take(&mut current)));
                used = 0;
            } else if separator > 0 {
                current.push(Span::styled(" ".repeat(separator), span.style));
                used += separator;
            } else {
                // No room for a gap: the token starts the row alone.
            }
            current.push(Span::styled(token.to_owned(), span.style));
            used += length;
            gap.clear();
            gap.push(' ');
            at_row_start = false;
        }
    }
    if !current.is_empty() {
        rows.push(Line::from(current));
    }
    if rows.is_empty() {
        rows.push(Line::default());
    }
    rows
}

/// The help overlay for the focused field, or `None` when it is not showing.
///
/// The card is drawn below the popup, at the popup's own width, with the
/// text wrapped to it. It is an overlay on the terminal rather than a row
/// inside the form, and it sizes itself to its content, so its height is
/// measured before it is placed.
fn help_overlay<'a>(
    popup: &AttendantPropertiesState,
    popup_area: Rect,
    inner: Rect,
    terminal: Rect,
    theme: &'a jinn_theme::Theme,
) -> Option<HelpCard<'a>> {
    // The template editor owns the terminal cursor while it is open; the
    // help would sit on top of the draft the user is typing into.
    if !popup.help_visible || popup.editor_original.is_some() {
        return None;
    }
    let card = help_card(theme);
    // The text is drawn inside the border, so it wraps a card narrower and
    // rows shorter than the card. Wrapping at the card's own width instead
    // would under-count the rows, and a card that is shorter than its text
    // runs off its own bottom border.
    let text_width = inner.width.saturating_sub(HELP_CARD_BORDER_CELLS);
    let lines = help_body_lines(popup.focus, text_width, theme);
    // The card is a border around its text, so its height is the text's
    // plus the two border rows — measured here rather than left to the
    // `Block`, because the card is placed before it is drawn and a height
    // that is only known at draw time is a height that cannot be placed.
    let height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2)
        .max(2);
    let y = place_help(&HelpPlacementInput {
        height,
        // The card is kept off the popup *including its border*, not just
        // its body: a card drawn over the popup's own top edge reads as a
        // rendering fault rather than as an overlay.
        popup_bottom: popup_area.y.saturating_add(popup_area.height),
        terminal_top: terminal.y,
        terminal_bottom: terminal.y.saturating_add(terminal.height),
    });
    let area = Rect {
        x: inner.x,
        y,
        width: inner.width,
        height,
    };
    let text = card.inner(area);
    Some(HelpCard {
        area,
        text,
        card,
        lines,
    })
}

/// The help card, placed: the area it occupies, the text area inside its
/// border, the frame that draws the border, and the lines the text is.
struct HelpCard<'a> {
    /// The card's whole area, border included.
    area: Rect,
    /// The area inside the border, where the text goes.
    text: Rect,
    /// The card's frame.
    card: Block<'a>,
    /// The card's text, already wrapped to `text`'s width.
    lines: Vec<Line<'static>>,
}

/// Where the help card goes: its height, and the bounds it must fit in.
struct HelpPlacementInput {
    /// The card's measured height.
    height: u16,
    /// One past the popup's last row, border included.
    popup_bottom: u16,
    /// The terminal's first row.
    terminal_top: u16,
    /// One past the terminal's last row.
    terminal_bottom: u16,
}

/// The card's top row: always below the popup, flush with its border.
///
/// Below is the card's only home. Above was the old placement, and a card
/// that flips sides as the user moves between fields is a card that jumps
/// under the cursor — the help is read, not aimed at, and a fixed place is
/// what lets a reader find the next field's help without hunting for it.
///
/// The card sits on the popup's last row. There was a blank row between
/// them once, to keep the two from reading as one block; the card's own
/// pink border does that job, and does it better than an empty row — a gap
/// between two framed things is a hole in the screen, not separation. A
/// terminal too short to hold the card below it draws from its own top row
/// and lets the last rows be cut: an overlay is laid over the screen, and
/// there is nowhere else to put it.
fn place_help(input: &HelpPlacementInput) -> u16 {
    let below = input
        .popup_bottom
        .min(input.terminal_bottom.saturating_sub(input.height));
    if below.saturating_add(input.height) <= input.terminal_bottom {
        below
    } else {
        // No room below. The card is drawn from the top of the terminal and
        // the terminal cuts it, rather than from the top of the room below
        // the popup: a card that starts above the popup covers the form it
        // is describing. Neither reads as an overlay.
        input.terminal_top
    }
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
fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> Option<&'a mut jinn_kernel::AppState> {
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
fn action<F>(cell: &AttendantPropertiesCell, f: F) -> ActionFn
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
fn row(
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

/// The `h`/`l` action: acts on whichever row the form cursor is on.
///
/// Four of the six rows are decided entirely from the popup's own cell, so
/// they go straight to [`AttendantPropertiesState::pick`]. The two set rows
/// are the exception: freezing one is a statement about the attendant's
/// present capabilities, and reading those means reaching `AppState` — the
/// same way [`save_attendant`] and [`commit_pending_to_session`] do.
///
/// The key walks the row rather than toggling it. Every other choice row in
/// the form moves one position per press and stops at its ends, and a row
/// that reads identically to those should not behave differently under the
/// same key: `h` on the leftmost choice is a no-op rather than a flip to the
/// rightmost, and `l` on the rightmost is a no-op rather than a re-freeze
/// that would recapture against a state the user has since changed.
///
/// The direction is still consumed by the *outcome*, which is what a
/// two-state row cannot avoid: moving onto Frozen is the freeze, whatever
/// came from.
fn pick_on(
    ctx: &mut ActionCtx<'_>,
    cell: &AttendantPropertiesCell,
    direction: PickDirection,
) -> IntentResult {
    let Some(field) = set_field_of(cell.read().focus) else {
        cell.update(|popup| popup.pick(direction));
        return IntentResult::empty();
    };
    let Some(attendant_id) = cell.read().session_id.clone() else {
        return IntentResult::empty();
    };
    let current = cell.read().set_mode_of(field);
    let Some(next) = next_set_mode(current, direction) else {
        // Already at the end the key points at. Unlike the cell-only rows,
        // this leaves the status line alone: there is nothing to report
        // about a key that walked to the end of its row, and the row's
        // current value is still on it.
        return IntentResult::empty();
    };
    let thawing = next == SetMode::Live;
    // A thaw has nothing to read: the capture is discarded and the
    // attendant's filter is left for the commit to leave alone.
    let permitted = if thawing {
        BTreeSet::new()
    } else {
        permitted_now(ctx, &attendant_id, field)
    };
    // A capture that came back empty is a set frozen to nothing, and the
    // commit writes it: an allow list naming nothing is a filter that
    // withholds every name, so nothing has to be reported about it. A glob
    // in the attendant's filter is the one thing a flip does silently
    // change, so that is what the line is for.
    let dropped = !thawing && contains_glob(ctx, &attendant_id, field);
    cell.update(|popup| {
        popup.set_mode(field, next, &permitted);
        if dropped {
            popup.report(PopupStatus::GlobDropped { field });
        }
    });
    IntentResult::empty()
}

/// The mode one position along a set row from `current`, or `None` when the
/// key points past the end.
///
/// The two choices run left to right — `live` then `frozen` — so the row is
/// a window with two positions in it and the keys move within it. Stopping
/// at the ends is what makes the row honest: `h` says "left", and on the
/// leftmost row there is nothing to its left.
fn next_set_mode(current: SetMode, direction: PickDirection) -> Option<SetMode> {
    match (direction, current) {
        (PickDirection::Right, SetMode::Live) => Some(SetMode::Frozen),
        (PickDirection::Left, SetMode::Frozen) => Some(SetMode::Live),
        _ => None,
    }
}

/// The set row a form field names, or `None` for the four rows that are
/// decided from the cell alone.
fn set_field_of(field: PropertyField) -> Option<SetField> {
    match field {
        PropertyField::ToolSet => Some(SetField::Tool),
        PropertyField::SkillSet => Some(SetField::Skill),
        PropertyField::Trigger
        | PropertyField::Behavior
        | PropertyField::PrepMode
        | PropertyField::SeedTemplate => None,
    }
}

/// The names the attendant currently permits for `field`.
///
/// The sources are the ones a picker seeds its rows from, narrowed by the
/// attendant's own filter and — for tools — by the provider gate, because a
/// name that is refused at dispatch however the filter is written is not
/// worth freezing in: it would make `jinn.toml` longer to read and change
/// nothing.
///
/// This is the same conjunction the context assembler applies, and it is
/// deliberately derived rather than read from a registry: a name the
/// attendant cannot use is not part of what freezing is meant to preserve.
fn permitted_now(
    ctx: &mut ActionCtx<'_>,
    attendant_id: &jinn_core_types::SessionId,
    field: SetField,
) -> BTreeSet<String> {
    let Some(state) = app(ctx) else {
        return BTreeSet::new();
    };
    let Some(session) = state.session.get(attendant_id) else {
        return BTreeSet::new();
    };
    match field {
        SetField::Skill => session
            .discovered_skills()
            .iter()
            .filter(|skill| session.is_skill_enabled(&skill.name))
            .map(|skill| skill.name.clone())
            .collect(),
        SetField::Tool => {
            let provider = session.model_selection().provider_name().to_owned();
            state
                .tool_registry()
                .map(|registry| {
                    registry
                        .read()
                        .tools_for_session(attendant_id)
                        .into_iter()
                        .filter(|def| session.is_tool_enabled(&def.name))
                        .filter(|def| def.available_for_provider(&provider))
                        .map(|def| def.name)
                        .collect()
                })
                .unwrap_or_default()
        }
    }
}

/// Whether the attendant's own filter for `field` holds a glob pattern.
///
/// Only the attendant's *own* filter is asked. An inherited parent pattern
/// is not the user's, and reporting a drop they did not make — or dropping
/// a pattern that never applied to this attendant — would put a message on
/// the status line about an edit that was not made.
fn contains_glob(
    ctx: &mut ActionCtx<'_>,
    attendant_id: &jinn_core_types::SessionId,
    field: SetField,
) -> bool {
    let Some(state) = app(ctx) else {
        return false;
    };
    let Some(session) = state.session.get(attendant_id) else {
        return false;
    };
    let filter = match field {
        SetField::Tool => session.tool_filter(),
        SetField::Skill => session.skill_filter(),
    };
    filter.is_some_and(|filter| filter.names.iter().any(|pattern| is_glob(pattern)))
}

/// Whether `pattern` is a glob rather than a plain name.
///
/// A pattern that would not compile as a glob is a literal — the same
/// reading `NameFilter` gives it when matching — so an uncompilable pattern
/// is not reported as one.
fn is_glob(pattern: &str) -> bool {
    pattern.contains(['*', '?', '[', '{'])
}

/// The `<c-s>` action: saves the popup's attendant to `jinn.toml`.
///
/// A new name saves on the first press. An existing name arms on the first
/// press and overwrites on the second, because the entry under that name
/// holds an attendant the user may have spent an afternoon building, and a
/// single stray key should not be able to replace it.
///
/// A session with no title cannot be saved: the title *is* the entry's
/// identity, and a session that never received a submission has none.
///
/// Every outcome is reported on the popup's status line, including the
/// refusals. A key that does nothing looks like a broken key, and the
/// popup is the surface the user is looking at when they press one.
fn save_attendant(ctx: &mut ActionCtx<'_>, cell: &AttendantPropertiesCell) -> IntentResult {
    let popup = cell.read().clone();
    let Some(attendant_id) = popup.session_id.clone() else {
        return IntentResult::empty();
    };
    // Read the config before the state borrow: `app` takes `ctx` mutably.
    let config = ctx.config.clone();
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    let Some(session) = state.session.get(&attendant_id) else {
        return IntentResult::empty();
    };
    let Some(name) = session
        .title()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
    else {
        return refuse_save(attendant_id, cell);
    };

    // The layer reads the live document, so a name collision is a property
    // of what is on disk rather than of anything this popup cached.
    // A malformed list is not an empty list: saving on top of it would
    // replace the user's entries with just this one, and the collision
    // check would have seen nothing. Read failures refuse the save.
    let mut entries =
        match config.get_list::<jinn_preferences_config::schemas::AttendantEntryConfig>() {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(err = ?error, "failed to read the saved attendants");
                cell.update(|p| {
                    p.report(PopupStatus::SaveFailed {
                        reason: "Cannot save: jinn.toml's saved attendants could not be read."
                            .to_owned(),
                    });
                });
                return IntentResult::empty();
            }
        };
    let collides = entries.iter().any(|existing| existing.name == name);
    if collides && !popup.save_armed {
        let armed = name.clone();
        cell.update(|p| {
            p.arm_save();
            p.report(PopupStatus::OverwriteArmed { name: armed });
        });
        return IntentResult::empty();
    }

    // Commit the popup's pending values to the session before reading it
    // back. Pressing save means wanting these settings, so the entry is
    // built from what the user is looking at — not from the pre-edit
    // session, which would write the old values and report success.
    //
    // `commit_pending_to_session` is the same path `<enter>` takes, so the
    // two ways of committing cannot drift apart.
    commit_pending_to_session(ctx, &popup, &attendant_id);

    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    let Some(session) = state.session.get(&attendant_id) else {
        return IntentResult::empty();
    };
    let entry = crate::saved_entry::entry_for_session(name.clone(), session);
    entries.retain(|existing| existing.name != entry.name);
    entries.push(entry);
    if let Err(error) =
        config.put_list::<jinn_preferences_config::schemas::AttendantEntryConfig>(&entries)
    {
        tracing::warn!(err = ?error, "failed to save the attendant to jinn.toml");
        let reason = format!("Could not save “{name}” — {}", describe_write_error(&error));
        cell.update(|p| p.report(PopupStatus::SaveFailed { reason }));
        return IntentResult::empty();
    }

    // Committed: the arm has served its purpose, and the popup's restore
    // point moves to what was just written so `<esc>` does not undo the
    // save it just reported as done.
    let saved = name;
    cell.update(|p| {
        p.disarm_save();
        p.commit_as_original();
        p.report(PopupStatus::Saved { name: saved });
    });

    // A save is a session edit, so it persists like any other. A freshly
    // created attendant was never interacted, and without this the write
    // below is silently dropped.
    IntentResult::empty().with_message(jinn_session_store_msg::PersistSession {
        session_id: attendant_id,
    })
}

/// Writes a properties popup's pending values onto its session.
///
/// Shared by `<enter>` (apply and close) and `<c-s>` (save and stay) so the
/// two commit paths cannot disagree about what "committed" means. Marks the
/// session interacted and touched, so a freshly created attendant persists.
///
/// The two set rows are written here rather than in the save path, because
/// `<enter>` alone has to leave the session holding what the panel showed.
/// A Live row writes nothing: the attendant inherits its parent's set, which
/// is what an unconfigured filter already means.
fn commit_pending_to_session(
    ctx: &mut ActionCtx<'_>,
    popup: &jinn_attendant_msg::AttendantPropertiesState,
    attendant_id: &jinn_core_types::SessionId,
) {
    let Some(state) = app(ctx) else {
        return;
    };
    let Some(session) = state.session.get_mut(attendant_id) else {
        return;
    };
    session.set_seed_template(popup.seed_template.input.clone());
    session.set_attendant_behavior(popup.pending_behavior);
    session.set_attendant_is_prepping(popup.pending_prep_mode);
    session.set_attendant_trigger(popup.pending_trigger);
    for field in [SetField::Tool, SetField::Skill] {
        // Only a row the user actually moved may write. An untouched row
        // opens as a reading of whatever filter the attendant already
        // carried -- a hand-written blocklist included -- and committing
        // that reading back unchanged is what keeps opening and saving an
        // untouched panel a no-op on every field.
        if !popup.set_touched(field) {
            continue;
        }
        let filter = committed_filter(popup, field);
        match field {
            SetField::Tool => session.set_tool_filter(filter),
            SetField::Skill => session.set_skill_filter(filter),
        }
    }
    // A fresh attendant was never interacted; without this the persist is
    // silently dropped.
    session.mark_interacted();
    session.touch();
}

/// What a set row commits to the session's filter.
///
/// A Live row commits an *unconfigured* filter, and that is the whole
/// reason the commit cannot simply skip a Live row. The row's mode is
/// derived from the session's filter — `OriginalValues::mode_of` reads an
/// allow-mode filter as Frozen — so leaving a thawed attendant's filter in
/// place left the session refusing everything outside the old allow list
/// while the panel said the set was live, and the next open of the panel
/// read Frozen again. There was no way back to Live. Writing the
/// unconfigured filter is what actually releases the set, and it means the
/// attendant inherits its parent's from there.
///
/// A row is never cleared because the attendant was *hand-written* as
/// frozen, though: an untouched popup commits the same unconfigured filter,
/// which is already what such an attendant inherits, so the file is left
/// alone and nothing widens behind the user's back. The freeze only ever
/// came from this panel, so this releases exactly what this panel put there.
///
/// A Frozen row whose capture is empty also commits an unconfigured
/// filter, which is the same release as a thaw. An empty allow list is read
/// as no filter at all, so persisting one would mark the attendant frozen
/// in `jinn.toml` while it went on inheriting everything — the one outcome
/// the row exists to prevent. That is why the refusal lives here, at the
/// write, rather than at the choice: refusing the mode instead, as this
/// once did, made the row unselectable on any attendant that had
/// discovered nothing yet, which is the default state of a fresh one.
fn committed_filter(
    popup: &jinn_attendant_msg::AttendantPropertiesState,
    field: SetField,
) -> Option<NameFilter> {
    // A captured set commits as an allow list over exactly those names.
    // An empty capture is a capture: it is a set frozen to nothing, and
    // it commits as an allow list naming nothing, which withholds every
    // name. That is the whole point of the row — refusing the empty case
    // left a freshly created attendant (which has the parent's filters
    // but none of its discovered skills) with no way to say so.
    // Live: no filter at all, so the attendant inherits its parent's.
    popup.pending_set(field).map(|names| NameFilter {
        mode: FilterMode::Allow,
        names: names.clone(),
    })
}

/// The user-facing half of a config write failure.
///
/// The report's own chain carries the path and the cause for a log reader;
/// the popup has room for one clause, and "the file could not be written"
/// is the part a user can act on.
fn describe_write_error(error: &error_stack::Report<jinn_config::ConfigError>) -> String {
    match error.current_context() {
        jinn_config::ConfigError::Storage { .. } => "jinn.toml could not be written.".to_owned(),
        _ => "jinn.toml could not be updated.".to_owned(),
    }
}

/// Tells the attendant why it was not saved, in its own chat log, and says
/// the same thing on the popup's status line.
///
/// The popup targets a highlighted session that is not necessarily the
/// active one, so the chat line goes to the attendant itself — a refusal
/// written into some other session's log would be both invisible and
/// misleading. The status line is here because the popup covers the chat
/// log while it is open, and the user pressing `<c-s>` is looking at the
/// popup, not behind it.
fn refuse_save(
    attendant_id: jinn_core_types::SessionId,
    cell: &AttendantPropertiesCell,
) -> IntentResult {
    const REASON: &str = "This attendant has no name yet — send it a message first.";
    cell.update(|p| {
        p.report(PopupStatus::SaveFailed {
            reason: format!("Cannot save: {REASON}"),
        });
    });
    IntentResult::empty().with_message(jinn_session_history_msg::PushChatEntry {
        session_id: attendant_id,
        entry: jinn_core_types::ChatEntry::error(format!("Cannot save: {REASON}")),
        pin: None,
    })
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

/// Attaches the template editor's keep/restore/clear rows on its own scope.
pub fn attach_seed_template_rows(routes: &KeyRoutes, cell: &AttendantPropertiesCell) {
    let editor_scope = attendant_seed_template_scope();

    routes.attach(row(
        "attendant-template-keep",
        editor_scope.clone(),
        "<enter>",
        "general",
        "keep the edited template",
        action(cell, |_, cell| {
            cell.update(AttendantPropertiesState::keep_template_edit);
            IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(
                jinn_attendant_msg::attendant_seed_template_scope(),
            ))
        }),
    ));
    routes.attach(row(
        "attendant-template-restore",
        editor_scope.clone(),
        "<esc>",
        "general",
        "restore the pre-editor template",
        action(cell, |_, cell| {
            cell.update(AttendantPropertiesState::cancel_template_edit);
            IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(
                jinn_attendant_msg::attendant_seed_template_scope(),
            ))
        }),
    ));
    routes.attach(row(
        "attendant-template-clear-or-leave",
        editor_scope,
        "<c-c>",
        "general",
        "clear the template, or leave when already empty",
        action(cell, |_, cell| {
            let was_empty = cell.update(|popup| {
                let was_empty = popup.seed_template.input.is_empty();
                if !was_empty {
                    // Clearing *is* the edit: the user wiped the text, so the
                    // draft stands rather than being restored away. This
                    // deliberately diverges from the rename popup, whose
                    // `<c-c>` preserves the input state for a later restore.
                    popup.seed_template = jinn_slices::LineInput::default();
                    popup.editor_original = None;
                }
                was_empty
            });
            if was_empty {
                IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(
                    jinn_attendant_msg::attendant_seed_template_scope(),
                ))
            } else {
                IntentResult::empty()
            }
        }),
    ));
}

/// Registers the template editor's input hook on the editor scope.
///
/// The properties scope captures no input, so typed characters reach the
/// template draft only through this registration.
pub fn register_seed_template_input_hook(routes: &KeyRoutes, cell: &AttendantPropertiesCell) {
    let hook = attendant_properties_input_hook(cell);
    routes.register_input_hook(&jinn_attendant_msg::attendant_seed_template_scope(), hook);
}
