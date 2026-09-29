//! The attendant properties popup — overlay geometry, view, and rows.
//!
//! The popup is a two-phase form. Its own scope is navigation-only: `j`/`k`
//! move the form cursor between the three fields, `h`/`l` pick a choice
//! within the focused field, and `i` (on the seed-template field) opens the
//! template editor on its own capturing scope. Every edit stays pending in
//! the popup's cell until `<enter>` commits all three fields to the session
//! together; `<esc>`/`<c-c>` restore the open-time snapshot and close.
//!
//! Both popup phases render the full form — only the top scope's overlay is
//! drawn, so the editor view re-assembles the same rows and adds the text
//! cursor. The shared row assembly lives in [`properties_view`].

use jinn_attendant_msg::{
    AttendantPropertiesState, PickDirection, PopupStatus, PropertyField,
    attendant_seed_template_scope,
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
type AttendantPropertiesCell = TypedCell<AttendantPropertiesState>;

/// Horizontal padding fraction for the popup (20% each side).
const POPUP_H_PAD_FRAC: f32 = 0.20;
/// Minimum popup width in cells.
const POPUP_MIN_WIDTH: u16 = 44;
/// Popup content height: three field rows, the status line, one footer line.
///
/// The help text is an overlay rather than a row in the form, so moving the
/// cursor no longer reflows the popup and this is a constant. The status
/// line is a row: it holds a message, and a message that reflowed the form
/// would move the fields out from under the cursor as it typed.
const POPUP_CONTENT_ROWS: u16 = 5;

/// Computes the centered properties popup rectangle: title, three field
/// rows, one hint line, and a keybind footer.
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
    if let Some((help_area, help_lines)) = help_overlay(&popup, area, inner, frame.area(), theme) {
        frame.render_widget(Clear, help_area);
        // The card's surface, painted across its whole area first: a
        // `Paragraph` paints only the cells a line's text covers, so a card
        // with a blank row in it would otherwise show a hole in its own
        // background rather than a blank row of card.
        frame.render_widget(
            Block::default().style(Style::default().bg(theme.user_block_bg)),
            help_area,
        );
        frame.render_widget(Paragraph::new(help_lines), help_area);
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
/// The draft's row depends on which fields rendered a hint line above it:
/// each field before the template contributes its own row, plus one more
/// when it is focused (its hint line).
fn properties_layout(popup: &AttendantPropertiesState, inner: Rect) -> PropertiesLayout {
    // The trigger and activation rows always sit above the template row, so
    // this no longer moves with the cursor.
    let rows_above = 2;
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
    for field in [
        PropertyField::Trigger,
        PropertyField::Activation,
        PropertyField::SeedTemplate,
    ] {
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
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the status line is read directly by the slice's tests"
    )
)]
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
    };
    Line::from(Span::styled(text, Style::default().fg(color)))
}

/// One hint as (key, description) — the keys the focused field responds to.
fn hints(focus: PropertyField) -> Vec<(&'static str, &'static str)> {
    let mut hints = match focus {
        PropertyField::Trigger | PropertyField::Activation => {
            vec![("h/l", "pick"), ("j/k", "field")]
        }
        PropertyField::SeedTemplate => vec![("i", "edit"), ("j/k", "field")],
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
fn field_line<'a>(
    popup: &AttendantPropertiesState,
    field: PropertyField,
    theme: &'a jinn_theme::Theme,
    layout: &PropertiesLayout,
) -> Line<'a> {
    let focused = popup.focus == field;
    let mut spans = vec![
        field_marker(focused, theme),
        field_name(field, focused, theme),
    ];
    match field {
        PropertyField::Trigger => spans.extend(choice_spans(
            jinn_attendant_msg::TRIGGER_CHOICES,
            &popup.pending_trigger,
            theme,
        )),
        PropertyField::Activation => spans.extend(choice_spans(
            jinn_attendant_msg::ACTIVATION_CHOICES,
            &popup.pending_activation,
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

/// The focused row's background: the user-message block, so the row reads
/// as a selection against a surface the user already knows rather than as a
/// new color introduced by this popup.
fn focused_row_style(theme: &jinn_theme::Theme) -> Style {
    Style::default().bg(theme.user_block_bg)
}

/// The focused-field marker: `▸` when focused, blank otherwise.
fn field_marker(focused: bool, theme: &jinn_theme::Theme) -> Span<'static> {
    Span::styled(
        marker_text(focused).to_owned(),
        Style::default().fg(if focused {
            theme.focus_accent
        } else {
            theme.primary_text
        }),
    )
}

/// The field's label, yellow only while its row is focused.
fn field_name(field: PropertyField, focused: bool, theme: &jinn_theme::Theme) -> Span<'static> {
    Span::styled(
        field_label(field),
        Style::default().fg(if focused {
            theme.focus_accent
        } else {
            theme.primary_text
        }),
    )
}

/// The choice spans for a choice row: the selected choice in the active
/// green, the rest in plain text, separated by muted slashes.
fn choice_spans<'a, T>(
    choices: &[(T, &'static str)],
    selected: &T,
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
            Line::from("When the attendant re-runs on its own:"),
            Line::from(""),
            line(
                "parent-completed",
                "runs after every response message in the parent",
            ),
            line("manual", "runs only when you trigger it yourself"),
        ],
        PropertyField::Activation => vec![
            Line::from("How the session's context is prepared before each run:"),
            Line::from(""),
            line("seed", "populate the session with data"),
            line("reset", "only pins survive activation"),
            line("preserve", "context is retained on activation"),
        ],
        PropertyField::SeedTemplate => vec![
            Line::from("Text injected on each activation, ahead of the previous report:"),
            Line::from(""),
            line("<prior report>", "replaced with the previous report"),
        ],
    }
}

/// The help card, styled: a header naming the field, then its body lines.
///
/// Every row carries the card's own background, and its foreground is
/// chosen by the row's *role* — header, body — so adding emphasis means
/// giving a line a color, not restating the surface it sits on. The body
/// lines are wrapped here rather than handed to the `Paragraph`, because the
/// card sizes itself to its content and must know its height before it can
/// be placed.
fn help_paragraph(
    field: PropertyField,
    inner_width: u16,
    theme: &jinn_theme::Theme,
) -> (Vec<Line<'static>>, u16) {
    // One cell of the popup's width is the card's own left margin, so the
    // text wraps a cell narrower than the row it is drawn into.
    let text_width = inner_width.saturating_sub(1).max(1);
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
    // its content.
    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    (lines, height.max(2))
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
            }
            current.push(Span::styled(token.to_owned(), span.style));
            used += length;
            gap = " ".to_owned();
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
fn help_overlay(
    popup: &AttendantPropertiesState,
    popup_area: Rect,
    inner: Rect,
    terminal: Rect,
    theme: &jinn_theme::Theme,
) -> Option<(Rect, Vec<Line<'static>>)> {
    // The template editor owns the terminal cursor while it is open; the
    // help would sit on top of the draft the user is typing into.
    if !popup.help_visible || popup.editor_original.is_some() {
        return None;
    }
    let (lines, height) = help_paragraph(popup.focus, inner.width, theme);
    let y = place_help(HelpPlacementInput {
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
    Some((area, lines))
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

/// The card's top row: always below the popup, one row clear of its border.
///
/// Below is the card's only home. Above was the old placement, and a card
/// that flips sides as the user moves between fields is a card that jumps
/// under the cursor — the help is read, not aimed at, and a fixed place is
/// what lets a reader find the next field's help without hunting for it.
///
/// One row of gap separates the card from the popup, so the two never read
/// as one block. A terminal too short to hold the card below it draws from
/// its own top row and lets the last rows be cut: an overlay is laid over
/// the screen, and there is nowhere else to put it.
fn place_help(input: HelpPlacementInput) -> u16 {
    let below = input
        .popup_bottom
        .saturating_add(1)
        .min(input.terminal_bottom.saturating_sub(input.height));
    if below.saturating_add(input.height) <= input.terminal_bottom {
        below
    } else {
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
                let text = text.clone();
                cell.update(move |s| {
                    for ch in text.chars() {
                        s.seed_template.insert_char(ch);
                    }
                });
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
        action(cell, |_, cell| {
            cell.update(|popup| popup.pick(PickDirection::Left));
            IntentResult::empty()
        }),
    ));
    routes.attach(row(
        "attendant-properties-pick-right",
        properties_scope.clone(),
        "l",
        "navigation",
        "pick the next choice",
        action(cell, |_, cell| {
            cell.update(|popup| popup.pick(PickDirection::Right));
            IntentResult::empty()
        }),
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

    // Committed: the arm has served its purpose, and a saved attendant is
    // unarmed whether or not the popup closes.
    let saved = name;
    cell.update(|p| {
        p.disarm_save();
        p.report(PopupStatus::Saved { name: saved });
    });
    IntentResult::empty()
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
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    let popup = cell.read().clone();
    let Some(attendant_id) = popup.session_id.clone() else {
        return IntentResult::empty();
    };
    let template = popup.seed_template.input.clone();
    let (activation, trigger) = (popup.pending_activation, popup.pending_trigger);
    let Some(session) = state.session.get_mut(&attendant_id) else {
        return IntentResult::empty();
    };
    session.set_seed_template(template);
    session.set_attendant_activation(activation);
    session.set_attendant_trigger(trigger);
    // A fresh attendant was never interacted; without this the persist
    // below is silently dropped.
    session.mark_interacted();
    session.touch();
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
