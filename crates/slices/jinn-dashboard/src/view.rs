//! The dashboard's VIEW artifact — draws the dashboard slice payload.
//!
//! Third of the contribution triple (STATE = [`DashboardState`] cell,
//! LOGIC = [`DashboardCanvasActor`](crate::canvas_actor::DashboardCanvasActor),
//! VIEW = [`DashboardView`]): a pure renderer over `&DashboardState`
//! plus the current theme, which stays in `AppState` because themes are
//! runtime-switchable application data, not slice payload.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Clear, HighlightSpacing, Paragraph, Row, Table, TableState};
use unicode_width::UnicodeWidthStr;

use crate::ActorLifecycle;
use crate::DashboardEntry;
use crate::DashboardState;
use jinn_slices::SlotKey;
use jinn_slices::view::SliceView;
use jinn_slices::view::ViewCx;
use jinn_theme::Theme;

/// Cells reserved for the State column: the longest state word plus
/// padding. Fitted to the words rather than guessed, so adding a state
/// that fits still lines up and one that does not is caught by a test.
const STATE_COL: u16 = 10;

/// Cells reserved for the Name column.
///
/// A partition-set entity's path carries a UUID key
/// (`jinn.discovery/<uuid>` is 51 cells), so no width shows every name.
/// 40 fits the common `jinn.*` paths and truncates the pathological ones
/// — which is what the selected-row overlay is for.
const NAME_COL: u16 = 40;

/// Renders the dashboard slice: one row per actor, selection highlight,
/// scroll window. Owns no domain state — every drawn value comes from
/// the slice and the theme.
///
/// The theme comes in through [`ViewCx`] — it is application data
/// (runtime-switchable), not slice payload.
#[derive(Debug)]
pub struct DashboardView {
    slot: SlotKey,
}

impl DashboardView {
    /// A view over the dashboard's canonical slice slot.
    #[must_use]
    pub fn new() -> Self {
        Self {
            slot: super::dashboard_slot(),
        }
    }
}

impl Default for DashboardView {
    fn default() -> Self {
        Self::new()
    }
}

impl SliceView for DashboardView {
    type Slice = DashboardState;

    fn slot(&self) -> SlotKey {
        self.slot.clone()
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, cx: &ViewCx<'_>, slice: &Self::Slice) {
        let theme = cx.theme;
        let actors = slice.actors();
        if actors.is_empty() {
            render_empty(frame, area, theme);
            return;
        }

        // Trunk parity: the scroll window is clamped per frame so the
        // selection stays visible in whatever viewport this frame has.
        // The slice actor owns the cell, so the clamp is the pure
        // read-side form — no write handle reaches the render path.
        let content_height = area.height.saturating_sub(1); // header row
        let offset = usize::from(slice.clamped_offset(content_height));
        let selected = slice.selected_index();

        let rows = build_rows(&actors, theme);
        // Notes absorbs whatever the two fixed columns leave. It is the
        // only elastic column: names are bounded and state words are
        // known, so a remainder spent on either would be wasted.
        let widths = [
            Constraint::Length(STATE_COL),
            Constraint::Length(NAME_COL),
            Constraint::Min(10),
        ];

        let header = Row::new(vec![
            Cell::from("State"),
            Cell::from("Name"),
            Cell::from("Notes"),
        ])
        .style(Style::default().add_modifier(Modifier::BOLD));

        let table = Table::new(rows, widths)
            .header(header)
            .column_spacing(2)
            .row_highlight_style(Style::default().fg(theme.focus_accent))
            .highlight_symbol("▸ ")
            .highlight_spacing(HighlightSpacing::Always);

        let mut table_state = TableState::default();
        table_state.select(Some(selected));
        *table_state.offset_mut() = offset;

        frame.render_stateful_widget(table, area, &mut table_state);

        // The overlay is drawn AFTER the table so it paints over the
        // truncated name of the selected row. Drawn second means it also
        // covers the Notes cell for that row, which is the cost of
        // breaking the 40-column maximum: a long name is worth more than
        // the status text it hides, and the full value is readable.
        if let Some(entry) = selected_truncated(&actors, selected, offset)
            && let Some(y) = row_y(area, selected, offset)
        {
            overlay_selected_name(frame, area, entry, theme, y);
        }
    }
}

/// The selected row, if it is visible AND its name does not fit the
/// column — the only case where the overlay earns its keep.
fn selected_truncated<'a>(
    actors: &[&'a DashboardEntry],
    selected: usize,
    offset: usize,
) -> Option<&'a DashboardEntry> {
    let entry = actors.get(selected)?;
    if selected < offset {
        // Scrolled out of the window: overlaying at a row the table
        // never drew would paint over the wrong actor.
        return None;
    }
    is_truncated(&entry.name, NAME_COL).then_some(entry)
}

/// Whether `name` is wider than `max` terminal cells.
fn is_truncated(name: &str, max: u16) -> bool {
    name.width() > usize::from(max)
}

/// Paints the selected row's full name over the row, breaking the
/// Name column's width so the whole value is readable.
///
/// `y` is the buffer row the table actually drew this entry on; the
/// caller computes it, because only the caller knows the scroll offset.
fn overlay_selected_name(
    frame: &mut Frame<'_>,
    area: Rect,
    entry: &DashboardEntry,
    theme: &Theme,
    y: u16,
) {
    let width = area.width.saturating_sub(NAME_X);
    if width == 0 || y >= area.height {
        return;
    }
    let overlay_area = Rect {
        x: NAME_X,
        y,
        width,
        height: 1,
    };
    // `Clear` first: the overlay is wider than its column, so without
    // erasing, the characters underneath (the rest of the name, then the
    // Notes cell) show through the gaps.
    frame.render_widget(Clear, overlay_area);

    // `Clear` also resets styling, which would truncate the cursor row's
    // highlight at the end of the name. Filling the row with
    // highlight-styled padding keeps the selected row reading as one
    // continuous bar rather than a highlighted stub with a dead tail.
    let highlight = Style::default().fg(theme.focus_accent);
    let pad = usize::from(width).saturating_sub(entry.name.width());
    let mut spans = vec![Span::styled(entry.name.as_str(), highlight)];
    if pad > 0 {
        spans.push(Span::styled(" ".repeat(pad), highlight));
    }
    let para = Paragraph::new(Line::from(spans));
    frame.render_widget(para, overlay_area);
}

/// The x offset at which the Name column starts: the highlight symbol
/// (2 cells), the State column, and the column spacing (2).
const HIGHLIGHT: u16 = 2;
const COLUMN_SPACING: u16 = 2;
const NAME_X: u16 = HIGHLIGHT + STATE_COL + COLUMN_SPACING;

/// The buffer row the table draws data row `index` on, accounting for
/// the header row and the scroll offset. `None` when the row is scrolled
/// out of the viewport — there is nothing to overlay in that case.
fn row_y(area: Rect, index: usize, offset: usize) -> Option<u16> {
    let relative = index.checked_sub(offset)?;
    let y = 1 + u16::try_from(relative).ok()?;
    (y < area.height).then_some(y)
}

/// Renders the empty-state placeholder.
fn render_empty(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    let para = Paragraph::new(Line::from(Span::styled(
        " No services registered.",
        Style::default().fg(theme.muted_text),
    )));
    frame.render_widget(para, area);
}

/// Builds the table rows from dashboard entries, applying per-lifecycle colors.
///
/// Three columns, in this order: State, Name, Notes. The Description
/// column is gone — of roughly forty-five actors on the fabric, exactly
/// one (the discord gateway) publishes a description, so the column was
/// 44 empty cells wide. Where a feature has prose to contribute it
/// belongs in Notes, which is the elastic column and the only one that
/// can hold a sentence.
fn build_rows<'a>(actors: &[&'a DashboardEntry], theme: &Theme) -> Vec<Row<'a>> {
    actors
        .iter()
        .map(|entry| {
            let (state_str, state_color) = lifecycle_display(entry.lifecycle, theme);
            let state_cell = Cell::from(state_str).style(Style::default().fg(state_color));

            let name_cell =
                Cell::from(entry.name.as_str()).style(Style::default().fg(theme.primary_text));

            // ONE Notes cell, two possible sources. A feature's own
            // status message wins when it has one: it is the more
            // specific, more current statement about that row. The
            // runtime's stop reason is the fallback, and is the only
            // thing shown for the majority of rows — the census covers
            // every actor on the fabric, most of which have no feature
            // status to report.
            let notes_str = entry
                .status_message
                .as_deref()
                .or(entry.stop_reason.as_deref())
                .unwrap_or("");
            let notes_cell = Cell::from(notes_str).style(Style::default().fg(theme.muted_text));

            Row::new(vec![state_cell, name_cell, notes_cell])
        })
        .collect()
}

/// Returns the display string and color for a lifecycle variant.
fn lifecycle_display(lifecycle: ActorLifecycle, theme: &Theme) -> (&'static str, Color) {
    match lifecycle {
        ActorLifecycle::Starting => ("Starting", theme.warning),
        ActorLifecycle::Running => ("Running", theme.success),
        // Muted, not the error color: a passivated actor is dormant and
        // will return on the next send. Painting it like a failure is
        // how a normal idle cycle reads as an incident.
        ActorLifecycle::Idle => ("Idle", theme.muted_text),
        ActorLifecycle::Dead => ("Dead", theme.error_text),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use crate::DashboardState;
    use crate::DashboardView;
    use jinn_slices::view::SliceView;
    use jinn_slices::view::ViewCx;
    use jinn_testutil::setup_term;
    use jinn_theme::default_theme;

    /// Collects the entire terminal buffer into a single string for substring
    /// assertions.
    fn buffer_string(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    #[rstest::rstest]
    #[test]
    fn dashboard_view_renders_rows_with_selection() {
        // Given a dashboard slice with two actors, the second selected.
        let mut slice = DashboardState::new();
        slice.mark_running("alpha", None);
        slice.mark_running("beta", Some("second".to_owned()));
        slice.select_next();
        let theme = default_theme();
        let cx = ViewCx { theme: &theme };

        // When rendering through the view.
        let (mut terminal, _area) = setup_term(80, 24);
        terminal
            .draw(|frame| {
                let mut view = DashboardView::new();
                let area = ratatui::layout::Rect::new(0, 0, 80, 24);
                view.render(frame, area, &cx, &slice);
            })
            .expect("render");

        // Then both rows are drawn and the marker sits on `beta`.
        let buf = buffer_string(&terminal);
        assert!(buf.contains("alpha"), "first row renders");
        assert!(buf.contains("beta"), "second row renders");
        assert!(buf.contains('▸'), "selection marker renders");
        assert!(buf.contains("Running"), "lifecycle column renders");
    }

    /// A passivated row must read as dormant, not as a failure. The
    /// rendered row is what a user actually sees, so the State column is
    /// pinned here rather than only in the state fold's tests.
    #[rstest::rstest]
    #[test]
    fn dashboard_view_renders_a_passivated_actor_as_idle() {
        // Given a dashboard slice whose only actor was passivated.
        let mut slice = DashboardState::new();
        slice.mark_idle("jinn.discovery/abc");
        let theme = default_theme();
        let cx = ViewCx { theme: &theme };

        // When rendering through the view.
        let (mut terminal, _area) = setup_term(80, 24);
        terminal
            .draw(|frame| {
                let mut view = DashboardView::new();
                let area = ratatui::layout::Rect::new(0, 0, 80, 24);
                view.render(frame, area, &cx, &slice);
            })
            .expect("render");

        // Then the row reads Idle, not Dead.
        let buf = buffer_string(&terminal);
        assert!(buf.contains("Idle"), "idle row renders: {buf}");
        assert!(!buf.contains("Dead"), "a dormant actor is not dead: {buf}");
    }

    /// A feature's status message and the runtime's stop reason share one
    /// Notes cell; the feature's wins when both are present.
    #[rstest::rstest]
    #[test]
    fn dashboard_view_prefers_the_feature_status_over_the_stop_reason() {
        // Given a stopped row that also carries a feature status.
        let mut slice = DashboardState::new();
        slice.mark_stopped("discord", "crashed (supervisor declined restart)");
        slice.set_status_message("discord", Some("reconnecting".to_owned()));
        let theme = default_theme();
        let cx = ViewCx { theme: &theme };

        // When rendering through the view.
        let (mut terminal, _area) = setup_term(80, 24);
        terminal
            .draw(|frame| {
                let mut view = DashboardView::new();
                let area = ratatui::layout::Rect::new(0, 0, 80, 24);
                view.render(frame, area, &cx, &slice);
            })
            .expect("render");

        // Then the Notes cell carries the status message, not the reason.
        let buf = buffer_string(&terminal);
        assert!(
            buf.contains("reconnecting"),
            "status message renders: {buf}"
        );
        assert!(
            !buf.contains("supervisor declined"),
            "the stop reason yields to the feature status: {buf}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn dashboard_view_renders_empty_placeholder_when_no_actors() {
        // Given an empty dashboard slice.
        let slice = DashboardState::new();
        let theme = default_theme();
        let cx = ViewCx { theme: &theme };

        // When rendering through the view.
        let (mut terminal, _area) = setup_term(80, 24);
        terminal
            .draw(|frame| {
                let mut view = DashboardView::new();
                let area = ratatui::layout::Rect::new(0, 0, 80, 24);
                view.render(frame, area, &cx, &slice);
            })
            .expect("render");

        // Then the placeholder is drawn.
        assert!(buffer_string(&terminal).contains("No services"));
    }

    #[rstest::rstest]
    #[test]
    fn clamp_scroll_keeps_selected_visible() {
        // Given a dashboard with 5 actors, selection at index 4, viewport 3.
        let mut state = DashboardState::new();
        for name in ["a", "b", "c", "d", "e"] {
            state.mark_running(name, None);
        }
        state.select_last(); // index 4
        assert_eq!(state.selected_index(), 4);

        // When clamping with viewport 3.
        state.clamp_scroll(3);

        // Then scroll_offset puts index 4 within the visible window.
        let visible_start = state.scroll_offset() as usize;
        let visible_end = visible_start + 3;
        assert!(
            (visible_start..visible_end).contains(&4),
            "selected index should be within visible window {visible_start}..{visible_end}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn render_clamps_the_offset_per_frame_without_mutating_the_slice() {
        // Given 8 actors with the last selected and a stored offset of 0
        // (no clamp has ever run on the cell).
        let mut slice = DashboardState::new();
        for name in ["a", "b", "c", "d", "e", "f", "g", "h"] {
            slice.mark_running(name, None);
        }
        slice.select_last();
        assert_eq!(slice.scroll_offset(), 0);
        let theme = default_theme();
        let cx = ViewCx { theme: &theme };

        // When rendering into a 5-row-tall viewport (1 header + 4 rows).
        let (mut terminal, _area) = setup_term(80, 5);
        terminal
            .draw(|frame| {
                let mut view = DashboardView::new();
                let area = ratatui::layout::Rect::new(0, 0, 80, 5);
                view.render(frame, area, &cx, &slice);
            })
            .expect("render");

        // Then the selected last row is drawn despite offset 0, and the
        // earliest rows scrolled out of the window.
        let buf = buffer_string(&terminal);
        assert!(buf.contains('h'), "selected last row renders: {buf}");
        assert!(
            !buf.contains(" a ") && !buf.contains("\u{2502}a"),
            "first row is scrolled out of the window: {buf}"
        );
        // And the read path left the slice's own offset untouched.
        assert_eq!(slice.scroll_offset(), 0, "render never mutates the slice");
    }
}

#[cfg(test)]
mod layout_tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::DashboardState;
    use jinn_slices::view::ViewCx;
    use jinn_testutil::setup_term;
    use jinn_theme::default_theme;

    fn buffer_string(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    /// The width every layout test renders at: wide enough that the
    /// three fixed columns fit with room to spare.
    const WIDTH: u16 = 120;

    /// Renders `slice` at `width` and returns the raw buffer text.
    fn render(slice: &DashboardState, width: u16, height: u16) -> String {
        let theme = default_theme();
        let cx = ViewCx { theme: &theme };
        let (mut terminal, _area) = setup_term(width, height);
        terminal
            .draw(|frame| {
                let mut view = DashboardView::new();
                view.render(frame, Rect::new(0, 0, width, height), &cx, slice);
            })
            .expect("render");
        buffer_string(&terminal)
    }

    /// The longest State word, which is what `STATE_COL` is fitted to.
    fn longest_state_word() -> u16 {
        [
            ActorLifecycle::Starting,
            ActorLifecycle::Running,
            ActorLifecycle::Idle,
            ActorLifecycle::Dead,
        ]
        .into_iter()
        .map(|l| u16::try_from(lifecycle_display(l, &default_theme()).0.width()).unwrap())
        .max()
        .expect("states exist")
    }

    #[rstest::rstest]
    fn the_state_column_is_wide_enough_for_every_state_word() {
        // Given the longest state word the view can render.
        let longest = longest_state_word();

        // Then the fixed column fits it, with room to spare.
        assert!(
            STATE_COL > longest,
            "STATE_COL {STATE_COL} does not fit the {longest}-cell word {longest} plus padding"
        );
    }

    #[rstest::rstest]
    fn a_name_at_the_column_limit_is_not_truncated() {
        // Given a name exactly the column width.
        let exact = "a".repeat(usize::from(NAME_COL));

        // Then it needs no overlay.
        assert!(!is_truncated(&exact, NAME_COL));
    }

    #[rstest::rstest]
    fn a_name_one_cell_over_the_limit_is_truncated() {
        // Given a name one cell too wide.
        let over = "a".repeat(usize::from(NAME_COL) + 1);

        // Then it does.
        assert!(is_truncated(&over, NAME_COL));
    }

    /// A partition-set entity's real path: `jinn.discovery/<uuid>` is 51
    /// cells, so the common case IS the truncated case.
    #[rstest::rstest]
    fn a_partition_entity_path_overflows_the_name_column() {
        // Given a real derived path.
        let path = "jinn.discovery/0199a3b2-1234-7abc-8def-0123456789ab";

        // Then it overflows and so gets the overlay treatment.
        assert!(path.width() > usize::from(NAME_COL));
        assert!(is_truncated(path, NAME_COL));
    }

    /// Display width, not char count: a CJK name occupies two cells per
    /// character, so 20 characters overflow a 40-cell column while 40 do
    /// not. Measuring bytes would get both wrong.
    #[rstest::rstest]
    fn truncation_is_measured_in_display_cells() {
        // Given a 20-character name that is 40 cells wide.
        let wide = "間".repeat(20);
        assert_eq!(wide.chars().count(), 20);
        assert_eq!(wide.width(), 40);

        // And one that is 42 cells wide.
        let wider = "間".repeat(21);
        assert_eq!(wider.width(), 42);

        // Then the 40-cell one fits and the 42-cell one does not.
        assert!(!is_truncated(&wide, NAME_COL));
        assert!(is_truncated(&wider, NAME_COL));
    }

    #[rstest::rstest]
    fn the_header_is_state_name_notes() {
        // Given a dashboard with one actor.
        let mut slice = DashboardState::new();
        slice.mark_running("inference", None);

        // When rendering.
        let buf = render(&slice, WIDTH, 10);

        // Then the columns read in the new order.
        // The buffer is read row-major, so the header is the first
        // `width` cells — not the first N characters.
        let header: String = buf.chars().take(usize::from(WIDTH)).collect();
        assert!(header.contains("State"), "header: {header}");
        assert!(header.contains("Name"), "header: {header}");
        assert!(header.contains("Notes"), "header: {header}");
        assert!(
            !header.contains("Description"),
            "the description column is gone: {header}"
        );
    }

    #[rstest::rstest]
    fn a_short_name_renders_in_full_without_an_overlay() {
        // Given a short-named actor that is selected.
        let mut slice = DashboardState::new();
        slice.mark_running("inference", None);
        slice.select_first();

        // When rendering.
        let buf = render(&slice, WIDTH, 10);

        // Then the name appears exactly as stored.
        assert!(buf.contains("inference"), "short name renders: {buf}");
    }

    #[rstest::rstest]
    fn a_long_name_on_the_selected_row_overlays_in_full() {
        // Given a selected actor whose name overflows the column.
        let mut slice = DashboardState::new();
        let long = "jinn.discovery/0199a3b2-1234-7abc-8def-0123456789ab";
        slice.mark_running(long, None);
        slice.select_first();

        // When rendering.
        let buf = render(&slice, 120, 10);

        // Then the FULL name is on screen, not the clipped prefix — this
        // is the whole point of the overlay.
        assert!(buf.contains(long), "the full name overlays: {buf}");
    }

    /// Without the overlay a long name is clipped by the column, so the
    /// tail is absent. This pins that the overlay is what makes it
    /// visible, rather than the name happening to fit.
    #[rstest::rstest]
    fn the_overlay_is_what_makes_a_long_name_readable() {
        // Given the same long-named actor, NOT selected.
        let mut slice = DashboardState::new();
        let long = "jinn.discovery/0199a3b2-1234-7abc-8def-0123456789ab";
        slice.mark_running(long, None);
        let other = "inference";
        slice.mark_running(other, None);
        slice.select_next(); // select `inference`, leaving the long row unselected

        // When rendering.
        let buf = render(&slice, 120, 10);

        // Then the long name is clipped (tail absent) and the selected
        // short one is intact.
        assert!(
            !buf.contains("0123456789ab"),
            "an unselected long name stays clipped: {buf}"
        );
        assert!(
            buf.contains(other),
            "the selected short name renders: {buf}"
        );
    }

    /// The overlay is only correct if it lands on the selected row. With
    /// two long names, only the selected one's full value may appear.
    #[rstest::rstest]
    fn only_the_selected_long_name_is_overlaid() {
        // Given two long-named actors, the second selected.
        let mut slice = DashboardState::new();
        let first = "jinn.discovery/aaaaaaaa-aaaa-7aaa-8aaa-aaaaaaaaaaaa";
        let second = "jinn.mcp.coordinator/12345678-1234-7abc-8def-abcdefabcdef";
        slice.mark_running(first, None);
        slice.mark_running(second, None);
        slice.select_next();

        // When rendering.
        let buf = render(&slice, 140, 10);

        // Then the selected one is whole and the unselected one is not.
        assert!(buf.contains(second), "selected name overlays: {buf}");
        assert!(
            !buf.contains("aaaaaaaaaaaa"),
            "an unselected long name stays clipped: {buf}"
        );
    }

    /// The overlay paints over Notes for the selected row. That is the
    /// accepted cost, but it must not touch any OTHER row.
    #[rstest::rstest]
    fn the_overlay_covers_only_its_own_row() {
        // Given a long-named selected row and a short row with notes.
        let mut slice = DashboardState::new();
        let long = "jinn.discovery/0199a3b2-1234-7abc-8def-0123456789ab";
        slice.mark_running(long, None);
        slice.set_status_message(long, Some("first-row-note".to_owned()));
        slice.mark_running("inference", None);
        slice.set_status_message("inference", Some("second-row-note".to_owned()));
        slice.select_first();

        // When rendering.
        let buf = render(&slice, 140, 10);

        // Then the other row's notes survive intact.
        assert!(
            buf.contains("second-row-note"),
            "the overlay must not reach the next row: {buf}"
        );
    }

    #[rstest::rstest]
    fn the_overlay_follows_the_scroll_offset() {
        // Given many actors so the list scrolls, with the selected one
        // far down the list.
        let mut slice = DashboardState::new();
        for i in 0..40 {
            slice.mark_running(format!("actor-{i}"), None);
        }
        let long = "jinn.discovery/0199a3b2-1234-7abc-8def-0123456789ab";
        slice.mark_running(long, None);
        slice.select_last();

        // When rendering into a short viewport.
        let buf = render(&slice, 140, 6);

        // Then the selected long name is still overlaid, because it is
        // scrolled INTO view rather than out of it.
        assert!(
            buf.contains(long),
            "the overlay tracks the scrolled row: {buf}"
        );
    }

    #[rstest::rstest]
    fn a_name_column_of_the_configured_width_leaves_the_rest_to_notes() {
        // Given a row with a long note.
        let mut slice = DashboardState::new();
        slice.mark_running("inference", None);
        slice.set_status_message("inference", Some("a-fairly-long-status-phrase".to_owned()));

        // When rendering at a wide terminal.
        let buf = render(&slice, 120, 10);

        // Then the note is drawn, not squeezed out by a fixed name width.
        assert!(
            buf.contains("a-fairly-long-status-phrase"),
            "notes absorb the remainder: {buf}"
        );
    }

    #[rstest::rstest]
    fn row_y_accounts_for_the_header_and_the_offset() {
        // Given a 10-row area and a 3-row scroll offset.
        let area = Rect::new(0, 0, 80, 10);

        // Then the first visible data row lands under the header.
        assert_eq!(row_y(area, 3, 3), Some(1));
        assert_eq!(row_y(area, 4, 3), Some(2));
        // And a row scrolled out of view has nowhere to draw.
        assert_eq!(row_y(area, 2, 3), None);
        // And a row past the bottom of the viewport is out too.
        assert_eq!(row_y(area, 20, 0), None);
    }

    /// The overlay must be INVISIBLE as a seam: its text carries the same
    /// foreground the table gives the selected row, so the revealed tail
    /// reads as part of the highlighted cursor row.
    #[rstest::rstest]
    fn the_overlay_text_uses_the_cursor_row_highlight() {
        // Given a selected long-named actor.
        let mut slice = DashboardState::new();
        let long = "jinn.discovery/0199a3b2-1234-7abc-8def-0123456789ab";
        slice.mark_running(long, None);
        slice.select_first();

        // When rendering.
        let theme = default_theme();
        let cx = ViewCx { theme: &theme };
        let (mut terminal, _area) = setup_term(WIDTH, 8);
        terminal
            .draw(|frame| {
                let mut view = DashboardView::new();
                view.render(frame, Rect::new(0, 0, WIDTH, 8), &cx, &slice);
            })
            .expect("render");

        // Then a cell beyond the 40-column name limit — the revealed tail,
        // which no table cell covers — carries the highlight colour, not
        // the neutral name colour.
        let buf = terminal.backend().buffer();
        let y = 1; // the selected data row
        let x = NAME_X + NAME_COL;
        assert!(
            x < WIDTH,
            "the tail must be on screen for this to mean anything"
        );
        assert_eq!(buf[(x, y)].fg, theme.focus_accent, "tail is highlighted");
        assert_ne!(
            buf[(x, y)].fg,
            theme.primary_text,
            "the overlay must not use the neutral name colour"
        );
    }

    /// The highlight bar must run to the end of the row. `Clear` resets
    /// styling, so without explicit padding the row would look highlighted
    /// only as far as the name.
    #[rstest::rstest]
    fn the_overlay_keeps_the_highlight_running_to_the_end_of_the_row() {
        // Given a selected long-named actor.
        let mut slice = DashboardState::new();
        let long = "jinn.discovery/0199a3b2-1234-7abc-8def-0123456789ab";
        slice.mark_running(long, None);
        slice.select_first();

        // When rendering.
        let theme = default_theme();
        let cx = ViewCx { theme: &theme };
        let (mut terminal, _area) = setup_term(WIDTH, 8);
        terminal
            .draw(|frame| {
                let mut view = DashboardView::new();
                view.render(frame, Rect::new(0, 0, WIDTH, 8), &cx, &slice);
            })
            .expect("render");

        // Then the last cell of the row is still highlighted, not reset.
        let buf = terminal.backend().buffer();
        let y = 1;
        let last = WIDTH - 1;
        assert_eq!(
            buf[(last, y)].fg,
            theme.focus_accent,
            "the highlight bar reaches the row's end"
        );
    }

    /// The overlay covers that row's Notes. That is the accepted cost of
    /// breaking the column width, pinned so a future change to it is
    /// deliberate.
    #[rstest::rstest]
    fn the_overlay_covers_the_selected_rows_own_notes() {
        // Given a selected long-named row that has notes.
        let mut slice = DashboardState::new();
        let long = "jinn.discovery/0199a3b2-1234-7abc-8def-0123456789ab";
        slice.mark_running(long, None);
        slice.set_status_message(long, Some("a-note-that-should-be-covered".to_owned()));
        slice.select_first();

        // When rendering.
        let buf = render(&slice, WIDTH, 8);

        // Then the note is not visible under the overlay.
        assert!(
            !buf.contains("a-note-that-should-be-covered"),
            "the overlay covers its own row's notes: {buf}"
        );
    }

    #[rstest::rstest]
    fn the_name_column_starts_after_the_highlight_state_and_spacing() {
        // Given the fixed column geometry.
        // Then the name begins past the highlight symbol, state, and gap.
        assert_eq!(NAME_X, HIGHLIGHT + STATE_COL + COLUMN_SPACING);
        // And it starts on screen in a normal-width terminal.
        const _: () = assert!(NAME_X < 80, "the name column fits a normal terminal");
    }
}
