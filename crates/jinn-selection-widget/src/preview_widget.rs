//! Preview selection widget - picker with a split-pane preview area.
//!
//! [`PreviewSelectionWidget`] renders a telescope-style popup overlay with a list
//! on one side and a preview pane on the other. On wide terminals the split is
//! vertical (list left 20%, preview right 80%). On narrow terminals the split is
//! horizontal (list on top, preview below).

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use std::sync::Arc;

use crate::preview_content::{PreviewCache, PreviewContent};
use crate::{PickerItem, PickerLayout, SelectionColors, SelectionState};

/// Popup width threshold for vertical (side-by-side) split.
/// Below this, the layout switches to horizontal (stacked) split.
pub const VERTICAL_SPLIT_MIN_WIDTH: u16 = 101;

/// What the preview pane solved on the last render: the selected item's
/// total preview line count and the pane's visible row count.
///
/// This is the ground truth a consumer clamps its stored scroll offset
/// against — the widget is the only component that knows how many lines
/// the pane actually draws (post-wrap, at the pane's real width), so the
/// report is filled per [`PreviewSelectionWidget::render`] rather than
/// re-derived by the consumer. `total_lines` counts drawn content only;
/// the pane's padding blanks are not content and are never counted. When
/// nothing is selected the pane draws nothing, so `total_lines` is 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewPaneReport {
    /// Total preview lines the pane draws from for the selected item,
    /// post-wrap, including placeholder lines an item renders for its own
    /// empty content. 0 when nothing is selected.
    pub total_lines: usize,
    /// Visible rows in the preview pane's body — the height the pane
    /// solved with, in both split modes.
    pub visible_rows: usize,
}

impl PreviewPaneReport {
    /// The largest scroll offset that still shows content: scrolling to
    /// this offset leaves the pane's last row on the last content line.
    /// This is the clamp bound for a consumer's stored scroll offset —
    /// clamping with it keeps at least one content line on screen.
    #[must_use]
    pub fn max_scroll(&self) -> usize {
        self.total_lines.saturating_sub(self.visible_rows)
    }
}

/// Fraction of the list area allocated to the list in vertical split.
pub const LIST_FRACTION: u16 = 20;

/// Number of visible list rows in horizontal split mode.
pub const HORIZONTAL_LIST_ROWS: u16 = 5;

/// Horizontal separator character.
const HORIZONTAL_SEPARATOR: char = '\u{2500}'; // ─

/// Vertical separator character.
const VERTICAL_SEPARATOR: char = '\u{2502}'; // │

/// Filter prompt displayed before the user's input text.
const PROMPT: &str = "> ";

/// Renders a picker popup with a preview pane.
///
/// Generic over any type implementing both [`PickerItem`] and [`PreviewContent`].
/// Use the builder pattern to customize the title, footer, and colors, then call
/// [`render`](Self::render).
pub struct PreviewSelectionWidget<'a, T>
where
    T: PickerItem + PreviewContent,
{
    /// Title displayed in the popup border.
    title: Line<'a>,
    /// The selection state to render.
    state: &'a SelectionState<T>,
    /// Optional footer lines rendered at the bottom of the popup, top to
    /// bottom. Set via [`Self::footer`] (one line) or [`Self::footers`]
    /// (several); the popup reserves one row per line bottom-up.
    footers: Vec<Line<'a>>,
    /// Theme-dependent colors.
    colors: SelectionColors,
    /// Optional style override for the border title.
    title_style: Option<Style>,
    /// Preview pane scroll offset.
    preview_scroll: usize,
    /// Optional preview-line cache. When set, the preview pane uses
    /// [`PreviewContent::preview_lines_cached`] to skip re-rendering lines
    /// already stored under the selected item's cache key.
    preview_cache: Option<&'a dyn PreviewCache>,
    /// Optional slot the render fills with the preview pane's geometry
    /// report. Set via [`Self::preview_report`].
    preview_report: Option<&'a mut Option<PreviewPaneReport>>,
}

impl<'a, T> PreviewSelectionWidget<'a, T>
where
    T: PickerItem + PreviewContent,
{
    /// Creates a new preview widget rendering the given selection state.
    pub fn new(state: &'a SelectionState<T>) -> Self {
        Self {
            title: Line::from(""),
            state,
            footers: Vec::new(),
            colors: SelectionColors::default(),
            title_style: None,
            preview_scroll: 0,
            preview_cache: None,
            preview_report: None,
        }
    }

    /// Sets the popup border title.
    #[must_use]
    pub fn title(mut self, title: Line<'a>) -> Self {
        self.title = title;
        self
    }

    /// Sets a single footer line rendered at the bottom of the popup.
    /// Replaces any previously set footers. For multiple footer lines use
    /// [`Self::footers`].
    #[must_use]
    pub fn footer(mut self, footer: Line<'a>) -> Self {
        self.footers = vec![footer];
        self
    }

    /// Sets multiple footer lines rendered at the bottom of the popup, top
    /// to bottom. Each line occupies one terminal row; the popup allocates
    /// one row per footer line bottom-up (more footers -> fewer content
    /// rows). Replaces any previously set footers.
    #[must_use]
    pub fn footers(mut self, footers: Vec<Line<'a>>) -> Self {
        self.footers = footers;
        self
    }

    /// Sets the theme-dependent colors for the widget.
    #[must_use]
    pub fn colors(mut self, colors: SelectionColors) -> Self {
        self.colors = colors;
        self
    }

    /// Sets an optional style override for the popup border title.
    #[must_use]
    pub fn title_style(mut self, style: Style) -> Self {
        self.title_style = Some(style);
        self
    }

    /// Sets the preview pane scroll offset.
    #[must_use]
    pub fn preview_scroll(mut self, scroll: usize) -> Self {
        self.preview_scroll = scroll;
        self
    }

    /// Supplies a preview-line cache so the preview pane reuses already-rendered
    /// lines instead of re-rendering every frame.
    #[must_use]
    pub fn preview_cache(mut self, cache: &'a dyn PreviewCache) -> Self {
        self.preview_cache = Some(cache);
        self
    }

    /// Supplies the slot [`render`](Self::render) fills with the preview
    /// pane's geometry report ([`PreviewPaneReport`]).
    ///
    /// The widget writes one report per render — the frame's preview solve
    /// for the selected item — into `*out`. Pass `&mut Option::None` per
    /// frame and read it after `render` returns. Without this builder the
    /// render behaves exactly as before and writes nothing.
    #[must_use]
    pub fn preview_report(mut self, out: &'a mut Option<PreviewPaneReport>) -> Self {
        self.preview_report = Some(out);
        self
    }

    /// Renders the preview selection popup into the given popup rectangle.
    ///
    /// The caller owns the geometry: `area` is the popup rectangle the slice's
    /// overlay geometry function produced (see [`compute_popup_rect`]).
    ///
    /// Returns what the widget solved: `popup` is the rect handed in, `inner`
    /// is the list pane's results rect — the pane's own input + separator
    /// rows are chrome and are excluded — in both split modes.
    pub fn render(self, frame: &mut Frame<'_>, area: Rect) -> PickerLayout {
        let popup_area = area;
        frame.render_widget(Clear, popup_area);

        // Destructure self to take ownership of individual fields.
        let Self {
            title,
            state,
            footers,
            colors,
            title_style,
            preview_scroll,
            preview_cache,
            preview_report,
        } = self;

        let block = {
            let mut b = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(colors.border))
                .title(title);
            if let Some(style) = title_style {
                b = b.title_style(style);
            }
            b
        };
        frame.render_widget(block, popup_area);

        let inner = {
            let b = Block::default().borders(Borders::ALL);
            b.inner(popup_area)
        };

        // Reserve one row at the bottom per footer line.
        let footer_rows = footers.len() as u16;
        let [content_area, footer_area] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(footer_rows)]).areas(inner);

        // Build a temporary struct holding the remaining fields for the split renderers.
        let mut borrowed = RenderCtx::<T> {
            state,
            footers: &footers,
            colors: &colors,
            preview_scroll,
            preview_report,
        };

        let results_area = if popup_area.width >= VERTICAL_SPLIT_MIN_WIDTH {
            borrowed.render_vertical_split(frame, content_area, preview_cache)
        } else {
            borrowed.render_horizontal_split(frame, content_area, preview_cache)
        };

        borrowed.render_footer(frame, footer_area);

        PickerLayout {
            popup: popup_area,
            inner: results_area,
        }
    }

    // Forward to RenderCtx methods below are replaced by the RenderCtx struct.
}

/// Borrowed rendering context used after the block/title are consumed.
struct RenderCtx<'a, T: PickerItem + PreviewContent> {
    /// Shared picker state.
    state: &'a SelectionState<T>,

    /// Footer lines, top to bottom.
    footers: &'a [Line<'a>],
    /// Color configuration.
    colors: &'a SelectionColors,
    /// Scroll offset for the preview pane.
    preview_scroll: usize,
    /// Slot the preview pane fills with its geometry report, if the caller
    /// asked for one.
    preview_report: Option<&'a mut Option<PreviewPaneReport>>,
}

impl<T: PickerItem + PreviewContent> RenderCtx<'_, T> {
    /// Renders the list and preview side by side (wide terminals).
    fn render_vertical_split(
        &mut self,
        frame: &mut Frame<'_>,
        inner: Rect,
        cache: Option<&dyn PreviewCache>,
    ) -> Rect {
        let [list_area, preview_area] = Layout::horizontal([
            Constraint::Percentage(LIST_FRACTION),
            Constraint::Percentage(100 - LIST_FRACTION),
        ])
        .areas(inner);

        let results_area = self.render_list(frame, list_area);

        // Vertical separator as the first column of the preview area.
        let [sep_area, preview_body] =
            Layout::horizontal([Constraint::Length(1), Constraint::Min(0)]).areas(preview_area);
        let sep_lines: Vec<Line<'static>> = std::iter::repeat_n(
            Line::from(VERTICAL_SEPARATOR.to_string()),
            sep_area.height as usize,
        )
        .collect();
        let sep_paragraph =
            Paragraph::new(sep_lines).style(Style::default().fg(self.colors.separator));
        frame.render_widget(sep_paragraph, sep_area);

        self.render_preview(frame, preview_body, cache);

        results_area
    }

    /// Renders the list on top and the preview below (narrow terminals).
    fn render_horizontal_split(
        &mut self,
        frame: &mut Frame<'_>,
        inner: Rect,
        cache: Option<&dyn PreviewCache>,
    ) -> Rect {
        let list_height = HORIZONTAL_LIST_ROWS + 2;
        let [list_area, sep_area, preview_area] = Layout::vertical([
            Constraint::Length(list_height),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .areas(inner);

        let results_area = self.render_list(frame, list_area);

        let separator = HORIZONTAL_SEPARATOR
            .to_string()
            .repeat(sep_area.width as usize);
        let sep_paragraph =
            Paragraph::new(separator).style(Style::default().fg(self.colors.separator));
        frame.render_widget(sep_paragraph, sep_area);

        self.render_preview(frame, preview_area, cache);

        results_area
    }

    /// Render the filter input and result list, returning the results
    /// sub-rect the rows rendered into (the input + separator rows are
    /// chrome and are not part of it).
    fn render_list(&self, frame: &mut Frame<'_>, area: Rect) -> Rect {
        let [input_area, sep_area, results_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .areas(area);

        let filter_text = format!("{}{}", PROMPT, self.state.filter());
        let filter_paragraph =
            Paragraph::new(filter_text).style(Style::default().fg(self.colors.filter_text));
        frame.render_widget(filter_paragraph, input_area);
        let cursor_col = input_area.x + (PROMPT.len() + self.state.cursor_pos()) as u16;
        frame.set_cursor_position((cursor_col, input_area.y));

        let separator = HORIZONTAL_SEPARATOR.to_string().repeat(area.width as usize);
        let sep_paragraph =
            Paragraph::new(separator).style(Style::default().fg(self.colors.separator));
        frame.render_widget(sep_paragraph, sep_area);

        self.render_results(frame, results_area);

        results_area
    }

    /// Render the filtered result rows.
    fn render_results(&self, frame: &mut Frame<'_>, area: Rect) {
        let max_visible = area.height as usize;
        let scroll_offset = self.state.scroll_offset();
        let selection = self.state.selection();
        let mut result_lines = Vec::with_capacity(max_visible);

        for row in 0..max_visible {
            let entry_idx = scroll_offset + row;
            if let Some(item) = self.state.filtered_item(entry_idx) {
                let is_selected = entry_idx == selection;
                let match_indices = self.state.filtered_match_indices(entry_idx).unwrap_or(&[]);
                let ranges = super::widget::fuzzy_bytes_to_ranges(match_indices);
                result_lines.push(item.render_row_with_highlight(is_selected, &ranges));
            } else {
                result_lines.push(Line::from(""));
            }
        }
        frame.render_widget(Paragraph::new(result_lines), area);
    }

    /// Renders the preview pane for the currently selected item, consulting
    /// the optional `cache` to avoid re-rendering unchanged content.
    ///
    /// Fills the caller's report slot (when set) from the same lines the
    /// pane is about to draw, before any scroll clamping — the report
    /// describes the content's extent, not what the clamp chose to show.
    fn render_preview(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        cache: Option<&dyn PreviewCache>,
    ) {
        let width = area.width as usize;
        let max_visible = area.height as usize;

        let lines = if let Some(item) = self.state.selected_item() {
            item.preview_lines_cached(width, cache)
        } else {
            Arc::new(Vec::new())
        };

        // Report from the unclamped content: the pane's own display clamp
        // below only chooses which slice shows, and must not shrink the
        // reported extent as the user scrolls.
        if let Some(out) = self.preview_report.as_deref_mut() {
            *out = Some(PreviewPaneReport {
                total_lines: lines.len(),
                visible_rows: max_visible,
            });
        }

        // Display clamp: never draw past the end of the content. The slice
        // clamps its stored scroll itself; this only guards the frame.
        let skip = self.preview_scroll.min(lines.len().saturating_sub(1));

        let visible: Vec<Line<'static>> =
            lines.iter().skip(skip).take(max_visible).cloned().collect();

        let mut padded = visible;
        while padded.len() < max_visible {
            padded.push(Line::from(""));
        }

        let paragraph = Paragraph::new(padded).style(Style::default().fg(self.colors.filter_text));
        frame.render_widget(paragraph, area);
    }

    /// Renders the footer line at the bottom of the popup, right-aligned.
    fn render_footer(&self, frame: &mut Frame<'_>, area: Rect) {
        if self.footers.is_empty() {
            return;
        }
        let rows = Layout::vertical(vec![Constraint::Length(1); self.footers.len()]).split(area);
        for (line, row) in self.footers.iter().zip(rows.iter()) {
            let paragraph = Paragraph::new(line.clone())
                .style(Style::default().fg(self.colors.footer))
                .right_aligned();
            frame.render_widget(paragraph, *row);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::SharedPreviewLines;
    use crate::{PickerItem, SelectionState, compute_popup_rect};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::text::Line;

    /// A simple test item implementing both PickerItem and PreviewContent.
    #[derive(Debug, Clone)]
    struct TestItem {
        name: String,
        body: String,
    }

    impl PickerItem for TestItem {
        fn display_label(&self) -> &str {
            &self.name
        }
        fn render_row(&self, _is_selected: bool) -> Line<'static> {
            Line::from(self.name.clone())
        }
    }

    impl PreviewContent for TestItem {
        fn preview_lines(&self, _width: usize) -> Vec<Line<'static>> {
            self.body
                .lines()
                .map(|l| Line::from(l.to_owned()))
                .collect()
        }
        fn cache_key(&self) -> Option<String> {
            Some(self.name.clone())
        }
    }

    fn make_state(items: Vec<TestItem>) -> SelectionState<TestItem> {
        SelectionState::with_items(items)
    }

    /// Renders the popup with a shared preview cache and returns the buffer rows.
    ///
    /// A fresh `TestBackend` per call is what makes a "cold" render cold: the
    /// cache is the only state carried between the two calls.
    fn render_rows(state: &SelectionState<TestItem>, cache: &dyn PreviewCache) -> Vec<String> {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal should build");
        terminal
            .draw(|frame| {
                let popup = compute_popup_rect(frame.area());
                PreviewSelectionWidget::new(state)
                    .title(Line::from(" Skills "))
                    .colors(SelectionColors::default())
                    .preview_cache(cache)
                    .render(frame, popup);
            })
            .expect("draw should succeed");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol().to_owned())
            .collect()
    }

    /// A cache keyed by the item name, standing in for the skill cache.
    #[derive(Default)]
    struct NamedCache {
        entries: std::cell::RefCell<std::collections::HashMap<(String, usize), SharedPreviewLines>>,
    }

    impl PreviewCache for NamedCache {
        fn get(&self, key: &str, width: usize) -> Option<SharedPreviewLines> {
            self.entries
                .borrow()
                .get(&(key.to_owned(), width))
                .map(Arc::clone)
        }
        fn insert(&self, key: String, width: usize, lines: SharedPreviewLines) {
            self.entries.borrow_mut().insert((key, width), lines);
        }
    }

    fn cached_state() -> SelectionState<TestItem> {
        make_state(vec![TestItem {
            name: "rust".to_owned(),
            body: "# Rust rules\n\nUse `wherror` for errors.".to_owned(),
        }])
    }

    /// A test item whose preview wraps at exactly the pane's width: one
    /// line per `width` characters, as a stand-in for word-wrapping.
    /// Proves the report reflects the pane's actual width.
    #[derive(Debug, Clone)]
    struct WidthTestItem {
        text: String,
    }

    impl PickerItem for WidthTestItem {
        fn display_label(&self) -> &'static str {
            "width item"
        }
        fn render_row(&self, _is_selected: bool) -> Line<'static> {
            Line::from("width item")
        }
    }

    impl PreviewContent for WidthTestItem {
        fn preview_lines(&self, width: usize) -> Vec<Line<'static>> {
            self.text
                .chars()
                .collect::<Vec<_>>()
                .chunks(width.max(1))
                .map(|chunk| Line::from(chunk.iter().collect::<String>()))
                .collect()
        }
        fn cache_key(&self) -> Option<String> {
            Some("width-item".to_owned())
        }
    }

    /// Renders with a fresh report slot and returns what the render filled.
    fn render_with_report<T>(
        state: &SelectionState<T>,
        backend: ratatui::backend::TestBackend,
        preview_scroll: usize,
    ) -> PreviewPaneReport
    where
        T: PickerItem + PreviewContent,
    {
        let report = std::cell::RefCell::new(None);
        {
            let mut terminal = Terminal::new(backend).expect("terminal should build");
            terminal
                .draw(|frame| {
                    let popup = compute_popup_rect(frame.area());
                    let filled = PreviewSelectionWidget::new(state)
                        .preview_scroll(preview_scroll)
                        .preview_report(&mut report.borrow_mut())
                        .render(frame, popup);
                    // Keep the PickerLayout return alive to document that the
                    // report rides its own channel, not the render's return.
                    let _ = filled;
                })
                .expect("draw should succeed");
        }
        report
            .into_inner()
            .expect("render should fill the report slot")
    }

    /// Renders the popup on the given backend, returning the report and the
    /// drawn buffer so a test can check what the pane actually drew.
    fn render_report_and_buffer<T>(
        state: &SelectionState<T>,
        backend: ratatui::backend::TestBackend,
        preview_scroll: usize,
    ) -> (PreviewPaneReport, ratatui::buffer::Buffer)
    where
        T: PickerItem + PreviewContent,
    {
        let report = std::cell::RefCell::new(None);
        let mut terminal = Terminal::new(backend).expect("terminal should build");
        terminal
            .draw(|frame| {
                let popup = compute_popup_rect(frame.area());
                let _filled = PreviewSelectionWidget::new(state)
                    .preview_scroll(preview_scroll)
                    .preview_report(&mut report.borrow_mut())
                    .render(frame, popup);
            })
            .expect("draw should succeed");
        let buffer = terminal.backend().buffer().clone();
        let filled = report
            .into_inner()
            .expect("render should fill the report slot");
        (filled, buffer)
    }

    /// An item that always renders one placeholder line, standing in for
    /// items whose empty content still draws a "(no preview)" placeholder.
    #[derive(Debug, Clone)]
    struct PlaceholderItem;

    impl PickerItem for PlaceholderItem {
        fn display_label(&self) -> &'static str {
            "placeholder"
        }
        fn render_row(&self, _is_selected: bool) -> Line<'static> {
            Line::from("placeholder")
        }
    }

    impl PreviewContent for PlaceholderItem {
        fn preview_lines(&self, _width: usize) -> Vec<Line<'static>> {
            vec![Line::from("(no preview)")]
        }
    }

    #[rstest::rstest]
    fn a_warm_cache_renders_identically_to_a_cold_one() {
        // Given a state and an empty cache.
        let state = cached_state();
        let cache = NamedCache::default();

        // When the popup is rendered cold, then again from the warm cache.
        let cold = render_rows(&state, &cache);
        let warm = render_rows(&state, &cache);

        // Then the drawn rows are byte-identical — sharing the line buffer
        // through the cache must not change a single cell.
        assert_eq!(
            cold, warm,
            "a warm cache must render exactly like a cold one"
        );
    }

    #[rstest::rstest]
    fn a_cleared_cache_renders_identically_to_a_populated_one() {
        // Given a cache warmed by a first render.
        let state = cached_state();
        let cache = NamedCache::default();
        let cold = render_rows(&state, &cache);

        // When the cache is cleared (as on a theme change) and it is re-rendered.
        cache.entries.borrow_mut().clear();
        let after_clear = render_rows(&state, &cache);

        // Then the output matches the cold render, proving a cleared cache
        // re-renders from source rather than serving stale theme-colored lines.
        assert_eq!(
            cold, after_clear,
            "a cleared cache must re-render identically"
        );
    }

    #[rstest::rstest]
    fn a_cache_hits_again_after_the_picker_is_reopened() {
        // Given a cache warmed by a render, with the popup then torn down.
        let state = cached_state();
        let cache = NamedCache::default();
        render_rows(&state, &cache);
        assert_eq!(
            cache.entries.borrow().len(),
            1,
            "first render populates the cache"
        );

        // When the popup is rebuilt from scratch and rendered again.
        let reopened = SelectionState::with_items(vec![TestItem {
            name: "rust".to_owned(),
            body: "# Rust rules\n\nUse `wherror` for errors.".to_owned(),
        }]);
        let after_reopen = render_rows(&reopened, &cache);

        // Then it renders the same rows, served from the surviving cache.
        let fresh = render_rows(&reopened, &NamedCache::default());
        assert_eq!(after_reopen, fresh, "reopening must render identically");
    }

    #[rstest::rstest]
    fn a_resized_terminal_renders_at_the_new_width() {
        // Given a cache warmed at the preview pane's own width.
        let state = cached_state();
        let cache = NamedCache::default();
        render_rows(&state, &cache);
        let first = cache
            .entries
            .borrow()
            .keys()
            .next()
            .cloned()
            .expect("the render populates the cache");
        assert_eq!(cache.entries.borrow().len(), 1, "one width cached so far");

        // When the same item is requested at a different width.
        let item = state.selected_item().expect("should have selection");
        item.preview_lines_cached(17, Some(&cache));

        // Then the new width is a distinct entry, so the pane re-wraps instead
        // of reusing lines laid out for the old width.
        let keys: Vec<(String, usize)> = cache.entries.borrow().keys().cloned().collect();
        assert_eq!(keys.len(), 2, "the new width is cached separately");
        assert_ne!(first.1, 17);
        assert!(keys.iter().any(|(_, w)| *w == 17));
    }

    #[rstest::rstest]
    fn preview_widget_renders_into_the_rect_it_is_given() {
        // Given a state and an explicit popup rect not derived from the frame.
        let state = cached_state();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal should build");
        let popup = Rect::new(4, 2, 30, 10);

        // When rendering into that rect.
        terminal
            .draw(|frame| {
                PreviewSelectionWidget::new(&state).render(frame, popup);
            })
            .expect("draw should succeed");

        // Then the border is drawn on the supplied rect's top-left corner.
        let buffer = terminal.backend().buffer().clone();
        let top_left = buffer.cell((popup.x, popup.y)).expect("top-left cell");
        assert_eq!(
            top_left.symbol(),
            "┌",
            "border should sit on the supplied rect's corner"
        );
    }

    #[rstest::rstest]
    fn vertical_split_used_on_wide_terminal() {
        // Given a wide popup rect (popup_width from 140-col terminal = 112).
        let area = Rect::new(0, 0, 140, 30);
        // When computing the popup rect.
        let popup = compute_popup_rect(area);

        // Then the popup is wide enough for vertical split.
        assert!(
            popup.width >= VERTICAL_SPLIT_MIN_WIDTH,
            "popup width {} < {}",
            popup.width,
            VERTICAL_SPLIT_MIN_WIDTH
        );
    }

    #[rstest::rstest]
    fn horizontal_split_used_on_narrow_terminal() {
        // Given a narrow terminal.
        let area = Rect::new(0, 0, 60, 30);
        // When computing the popup rect.
        let popup = compute_popup_rect(area);

        // Then the popup is too narrow for vertical split.
        assert!(popup.width < VERTICAL_SPLIT_MIN_WIDTH);
    }

    #[rstest::rstest]
    fn preview_lines_called_for_selected_item() {
        // Given a state with an item that has preview content.
        let state = make_state(vec![TestItem {
            name: "test".to_owned(),
            body: "Hello preview".to_owned(),
        }]);
        let selected = state.selected_item().expect("should have selection");

        // When getting preview lines.
        let lines = selected.preview_lines(40);

        // Then the body content is returned.
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].to_string(), "Hello preview");
    }

    #[rstest::rstest]
    fn preview_lines_empty_for_empty_body() {
        // Given an item with no body.
        let item = TestItem {
            name: "empty".to_owned(),
            body: String::new(),
        };

        // When getting preview lines.
        let lines = item.preview_lines(40);

        // Then no lines are returned.
        assert_eq!(lines, [] as [ratatui::prelude::Line<'_>; 0]);
    }

    #[rstest::rstest]
    fn picker_layout_popup_is_the_rect_handed_in() {
        // Given a preview widget rendered into an explicit popup rect.
        let state = cached_state();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal should build");
        let popup = Rect::new(4, 2, 30, 10);
        let report = std::cell::Cell::new(None);

        // When rendering into that rect.
        terminal
            .draw(|frame| {
                report.set(Some(
                    PreviewSelectionWidget::new(&state).render(frame, popup),
                ));
            })
            .expect("draw should succeed");

        // Then the report's popup is the rect handed in, verbatim.
        let layout = report.into_inner().expect("render reports a PickerLayout");
        assert_eq!(layout.popup, popup);
    }

    #[rstest::rstest]
    fn picker_layout_inner_height_is_the_list_rows_in_stacked_mode() {
        // Given a state rendered into the popup computed on a narrow terminal.
        let state = cached_state();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal should build");
        let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
        assert!(
            popup.width < VERTICAL_SPLIT_MIN_WIDTH,
            "the solve must be the stacked mode"
        );
        let report = std::cell::Cell::new(None);

        // When rendering.
        terminal
            .draw(|frame| {
                report.set(Some(
                    PreviewSelectionWidget::new(&state).render(frame, popup),
                ));
            })
            .expect("draw should succeed");
        let layout = report.into_inner().expect("render reports a PickerLayout");

        // Then inner is the fixed pane's result rows: the pane's input and
        // separator rows are chrome, so the height is HORIZONTAL_LIST_ROWS,
        // not the pane's full HORIZONTAL_LIST_ROWS + 2.
        assert_eq!(layout.inner.height, HORIZONTAL_LIST_ROWS);
    }

    #[rstest::rstest]
    fn picker_layout_inner_height_excludes_pane_chrome_in_side_by_side_mode() {
        // Given a state rendered into the popup computed on a wide terminal.
        let state = cached_state();
        let backend = TestBackend::new(140, 30);
        let mut terminal = Terminal::new(backend).expect("terminal should build");
        let popup = compute_popup_rect(Rect::new(0, 0, 140, 30));
        assert!(
            popup.width >= VERTICAL_SPLIT_MIN_WIDTH,
            "the solve must be the side-by-side mode"
        );
        let report = std::cell::Cell::new(None);

        // When rendering with no footers.
        terminal
            .draw(|frame| {
                report.set(Some(
                    PreviewSelectionWidget::new(&state).render(frame, popup),
                ));
            })
            .expect("draw should succeed");
        let layout = report.into_inner().expect("render reports a PickerLayout");

        // Then inner is the left pane's result rows: the popup borders (2)
        // and the pane's own input + separator chrome (2) are excluded.
        assert_eq!(layout.inner.height, popup.height - 4);
    }

    #[rstest::rstest]
    fn picker_layout_inner_contains_the_drawn_result_rows() {
        // Given a state whose first item is selected.
        let state = cached_state();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal should build");
        let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
        let report = std::cell::Cell::new(None);

        // When rendering.
        terminal
            .draw(|frame| {
                report.set(Some(
                    PreviewSelectionWidget::new(&state).render(frame, popup),
                ));
            })
            .expect("draw should succeed");
        let layout = report.into_inner().expect("render reports a PickerLayout");

        // Then the drawn result rows land inside inner: the selected item's
        // name starts on inner's first row.
        let buffer = terminal.backend().buffer().clone();
        let first_row_cell = buffer
            .cell((layout.inner.x, layout.inner.y))
            .expect("first results cell");
        assert_eq!(
            first_row_cell.symbol(),
            "r",
            "the item 'rust' starts the rows"
        );
    }

    #[rstest::rstest]
    fn preview_report_total_lines_match_the_drawn_wrapped_lines() {
        // Given an item whose preview wraps at exactly the pane's width.
        let text = "a".repeat(200);
        let state = SelectionState::with_items(vec![WidthTestItem { text: text.clone() }]);
        let backend = TestBackend::new(140, 40);
        let popup = compute_popup_rect(Rect::new(0, 0, 140, 40));
        assert!(
            popup.width >= VERTICAL_SPLIT_MIN_WIDTH,
            "the solve must be the side-by-side mode"
        );

        // When rendering with a report slot.
        let (report, buffer) = render_report_and_buffer(&state, backend, 0);

        // Then total_lines is exactly what the pane draws at its width: the
        // text chunked at the pane body's own column count — the popup
        // inner minus borders, the list's 80/20 share, and the separator.
        let inner_width = usize::from(popup.width) - 2;
        let pane_width = inner_width * usize::from(100 - LIST_FRACTION) / 100 - 1;
        let expected = text.chars().count().div_ceil(pane_width);
        assert_eq!(
            report.total_lines, expected,
            "total_lines must be the wrapped count at the pane's width"
        );
        // And every pane row after the wrapped body is padding, proving the
        // count came from drawn content, not the padded buffer.
        let pad_row =
            usize::from(popup.y + popup.height) - 1 - (report.visible_rows - expected).max(1);
        let pad_cell = buffer
            .cell((
                popup.x + popup.width - 2,
                u16::try_from(pad_row).expect("u16 row"),
            ))
            .expect("padding cell");
        assert_eq!(pad_cell.symbol(), " ", "below the body the pane pads");
    }

    #[rstest::rstest]
    fn preview_report_wrap_count_depends_on_the_pane_width() {
        // Given one long item rendered on a wide frame and a narrow frame,
        // so the pane body width differs between the two renders.
        let state = SelectionState::with_items(vec![WidthTestItem {
            text: "a".repeat(200),
        }]);
        let wide = render_with_report(&state, TestBackend::new(140, 40), 0);
        let narrow = render_with_report(&state, TestBackend::new(70, 24), 0);

        // When reading each report's line count.
        let counts = [wide.total_lines, narrow.total_lines];

        // Then the counts differ — the report reflects the pane's actual
        // width, not a fixed line count.
        assert_ne!(
            counts[0], counts[1],
            "the same item must report different counts at different pane widths"
        );
    }

    #[rstest::rstest]
    fn preview_report_total_lines_match_short_content() {
        // Given a single-line item.
        let state = make_state(vec![TestItem {
            name: "test".to_owned(),
            body: "Hello preview".to_owned(),
        }]);
        let backend = TestBackend::new(80, 24);

        // When rendering with a report slot.
        let (report, _) = render_report_and_buffer(&state, backend, 0);

        // Then total_lines is 1 and max_scroll is 0 — the content is shorter
        // than any pane, so nothing is scrollable.
        assert_eq!(report.total_lines, 1, "one drawn content line");
        assert_eq!(report.max_scroll(), 0, "short content never scrolls");
    }

    #[rstest::rstest]
    fn preview_report_is_zero_when_nothing_is_selected() {
        // Given a state with no items at all.
        let state = SelectionState::<TestItem>::new();
        let backend = TestBackend::new(80, 24);

        // When rendering with a report slot.
        let (report, _) = render_report_and_buffer(&state, backend, 0);

        // Then the report names 0 content lines and max_scroll 0.
        assert_eq!(report.total_lines, 0, "no selection draws no content");
        assert_eq!(report.max_scroll(), 0, "no content means nothing to scroll");
        // And the pane still reports its real body height.
        assert_eq!(report.visible_rows, 12, "the pane keeps its body rows");
    }

    #[rstest::rstest]
    fn preview_report_counts_placeholder_lines_an_item_draws() {
        // Given an item whose empty content still renders one placeholder line.
        let state = SelectionState::with_items(vec![PlaceholderItem]);
        let backend = TestBackend::new(80, 24);

        // When rendering with a report slot.
        let (report, _) = render_report_and_buffer(&state, backend, 0);

        // Then the placeholder counts as drawn content.
        assert_eq!(report.total_lines, 1, "a drawn placeholder is content");
    }

    #[rstest::rstest]
    fn preview_report_visible_rows_match_pane_height_in_stacked_mode() {
        // Given a narrow frame solving the horizontal split.
        let state = cached_state();
        let backend = TestBackend::new(70, 24);
        let popup = compute_popup_rect(Rect::new(0, 0, 70, 24));
        assert!(
            popup.width < VERTICAL_SPLIT_MIN_WIDTH,
            "the solve must be the stacked mode"
        );

        // When rendering with a report slot.
        let (report, _) = render_report_and_buffer(&state, backend, 0);

        // Then visible_rows is the preview pane body's height: the popup
        // inner height minus the list pane's full band (HORIZONTAL_LIST_ROWS
        // + 2 — its input and separator chrome live inside that band) and
        // the separator row.
        assert_eq!(report.visible_rows, 12);
        assert_eq!(
            report.visible_rows,
            usize::from(popup.height) - 2 - usize::from(HORIZONTAL_LIST_ROWS) - 2 - 1
        );
    }

    #[rstest::rstest]
    fn preview_report_visible_rows_match_pane_height_in_side_by_side_mode() {
        // Given a wide frame solving the vertical split.
        let state = cached_state();
        let backend = TestBackend::new(140, 40);
        let popup = compute_popup_rect(Rect::new(0, 0, 140, 40));
        assert!(
            popup.width >= VERTICAL_SPLIT_MIN_WIDTH,
            "the solve must be the side-by-side mode"
        );

        // When rendering with a report slot.
        let (report, _) = render_report_and_buffer(&state, backend, 0);

        // Then visible_rows is the preview pane body's height: the popup
        // inner height. The separator is a column, so it narrows the pane's
        // width and takes none of its height.
        assert_eq!(report.visible_rows, 32);
        assert_eq!(report.visible_rows, usize::from(popup.height) - 2);
    }

    #[rstest::rstest]
    fn preview_report_zero_lines_when_nothing_is_selected_in_both_modes() {
        // Given an empty state rendered once per split mode.
        let empty = SelectionState::<TestItem>::new();
        let stacked = render_with_report(&empty, TestBackend::new(70, 24), 0);
        let side_by_side = render_with_report(&empty, TestBackend::new(140, 40), 0);

        // When comparing the two reports' line counts.
        let lines = [stacked.total_lines, side_by_side.total_lines];

        // Then both are 0 — no selection draws no content in either mode.
        assert!(
            lines.iter().all(|&l| l == 0),
            "no selection must report 0 lines in both split modes"
        );
    }

    #[rstest::rstest]
    fn preview_pane_display_clamp_holds_with_an_oversized_scroll() {
        // Given a 1-line item with a scroll offset far past the content.
        let state = make_state(vec![TestItem {
            name: "test".to_owned(),
            body: "only line".to_owned(),
        }]);
        let backend = TestBackend::new(80, 24);

        // When rendering with an oversized scroll.
        let (report, buffer) = render_report_and_buffer(&state, backend, 5_000);

        // Then the clamp shows the content instead of blank rows: the only
        // content line is drawn inside the pane body.
        let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
        let pane_body_row =
            popup.y + popup.height - 1 - u16::try_from(report.visible_rows).expect("u16 rows");
        let first_body_cell = buffer
            .cell((popup.x + 1, pane_body_row))
            .expect("pane body cell");
        assert_eq!(
            first_body_cell.symbol(),
            "o",
            "the clamped pane's top row is the content line"
        );
    }

    #[rstest::rstest]
    fn preview_report_is_unaffected_by_the_display_clamp() {
        // Given a long item (two lines) with an oversized scroll.
        let state = make_state(vec![TestItem {
            name: "test".to_owned(),
            body: "first\nsecond".to_owned(),
        }]);
        let backend = TestBackend::new(80, 24);

        // When rendering with an oversized scroll.
        let (report, _) = render_report_and_buffer(&state, backend, 5_000);

        // Then the report still names the content's full extent — the clamp
        // chooses what shows, it does not shrink what is reported.
        assert_eq!(report.total_lines, 2, "report reflects content, not clamp");
        assert_eq!(report.max_scroll(), 0, "max_scroll is the unclamped bound");
    }
}
