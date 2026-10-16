//! The `?` help card — a detached-looking sub-widget the popup's own
//! view draws over the market table.
//!
//! `?` flips [`MarketUiState::help_visible`] (the input hook owns the
//! key); while it is set the popup's view places this card below itself
//! and draws it: Clear, then the bordered card surface, then the text.
//! The card documents everything the popup puts on screen — every
//! column and flag glyph, the projection's mix presets, every keybind,
//! the detail-pane fields, the "updated X ago" stamps — and credits the
//! performance data's source.
//!
//! The shape follows the attendant's properties help card: a
//! variable-height document measured against the popup's inner width,
//! wrapped here (the card must know its height before it can be
//! placed), and painted with theme-semantic colors — explanation text
//! in the theme's text colors, key and column names in the accent the
//! table itself wears, the card's surface the theme's info-popup
//! background.

use jinn_model_market_msg::MIX_PRESETS;
use jinn_model_market_msg::MarketUiState;
use jinn_theme::Theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use unicode_segmentation::UnicodeSegmentation;

#[cfg(test)]
#[path = "help_tests.rs"]
mod help_tests;

/// The cells the card's border takes off its own width: one on each
/// side. The text is drawn inside them.
const CARD_BORDER_CELLS: u16 = 2;

/// The accent the table itself wears: its title, its header, its footer
/// keys. Key and column names in the card take it, so the card and the
/// table agree on which words are names and which are explanation.
fn name_style(theme: &Theme) -> Style {
    Style::default().fg(theme.popup_title)
}

/// The card's explanation text.
fn said_style(theme: &Theme) -> Style {
    Style::default().fg(theme.primary_text)
}

/// The card's secondary notes: caveats, the attribution sentence.
fn note_style(theme: &Theme) -> Style {
    Style::default().fg(theme.muted_text)
}

/// One documented thing: its name in the table's accent, then the
/// explanation after it.
fn entry(name: &str, description: &str, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("{name}:"), name_style(theme)),
        Span::styled(format!(" {description}"), said_style(theme)),
    ])
}

/// A secondary note under the entries it qualifies.
fn note(text: &str, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(text.to_owned(), note_style(theme)),
    ])
}

/// A section: one blank row of the author's spacing, then its title in
/// the accent.
fn section(title: &str, theme: &Theme) -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(Span::styled(
            title.to_owned(),
            name_style(theme).add_modifier(Modifier::BOLD),
        )),
    ]
}

/// The presets `m` cycles, in the status line's short form, rendered
/// from the same constant the cycle reads — the card cannot drift from
/// the behavior.
fn mix_preset_summary() -> String {
    let presets: Vec<String> = MIX_PRESETS
        .iter()
        .map(|(input, hit, output)| {
            format!(
                "{}k in @{}% cache \u{b7} {}k out",
                input / 1_000,
                (hit * 100.0).round() as u32,
                output / 1_000
            )
        })
        .collect();
    presets.join(" / ")
}

/// The Columns section: every header the table renders, and the rule
/// each column's `—` states.
fn column_section(theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = section("Columns", theme);
    lines.extend([
        entry(
            "MODEL",
            "author/slug, plus a dimmed :variant — the full id is what <enter> switches to.",
            theme,
        ),
        entry("IN/M", "prompt price, $ per million input tokens.", theme),
        entry(
            "OUT/M",
            "completion price, $ per million output tokens.",
            theme,
        ),
        entry(
            "CACHE/M",
            "cached-input read price. \u{2014} means the listing publishes no cache price: \
             every input token bills at the IN/M prompt price, and PROJ still costs the \
             row fully.",
            theme,
        ),
        entry(
            "PROJ \u{a2}",
            &format!(
                "projected \u{a2}/turn under the active mix: \
                 I\u{b7}(hit\u{b7}cache + (1\u{2212}hit)\u{b7}prompt) + O\u{b7}completion, \
                 recomputed every frame so m re-sorts live. Presets: {}.",
                mix_preset_summary()
            ),
            theme,
        ),
        entry(
            "ECI",
            "the Epoch Capabilities Index — Epoch AI's cross-benchmark capability score. \
             \u{2014} means the listing matched no ECI row (flagged #).",
            theme,
        ),
        entry(
            "FLAGS",
            "\u{2020} conditional pricing overrides \u{b7} f a :free listing \u{b7} \
             # no ECI data for this model.",
            theme,
        ),
    ]);
    lines
}

/// The Keys section: every keybind the popup honors, in the footer's
/// order, with the arrow keys it binds quietly listed beside their
/// `j`/`k` twins.
fn key_section(theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = section("Keys", theme);
    lines.extend([
        entry(
            "<leader>so",
            "open the market from any normal-mode scope.",
            theme,
        ),
        entry(
            "j/k, \u{2191}/\u{2193}",
            "move the highlight; the detail pane follows.",
            theme,
        ),
        entry(
            "<pgup>/<pgdn>",
            "page the highlight by one viewport.",
            theme,
        ),
        entry(
            "h/l",
            "move the sort column: PROJ, IN/M, OUT/M, CACHE/M, ECI, MODEL.",
            theme,
        ),
        entry("s", "flip the sort direction.", theme),
        entry(
            "m",
            "cycle the cost-mix preset (the status line shows the active one); \
             PROJ and the row order re-sort live.",
            theme,
        ),
        entry(
            "f/i",
            "focus the filter bar; typing fuzzy-narrows the rows.",
            theme,
        ),
        entry(
            "<esc>",
            "hand focus back to the table from the filter; close the popup from the table.",
            theme,
        ),
        entry(
            "<enter>",
            "switch the session to the highlighted model.",
            theme,
        ),
        entry(
            "<c-r>",
            "drop every cache — models, endpoints, epoch — and refetch everything.",
            theme,
        ),
        entry("?", "toggle this help.", theme),
    ]);
    lines
}

/// The Detail pane section: the fields the highlighted row's endpoints
/// render, and the auth-gated ones.
fn detail_section(theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = section("Detail pane (the highlighted model)", theme);
    lines.extend([
        entry(
            "endpoints",
            "one row per endpoint: routing tag, provider name, and that endpoint's \
             IN/M, OUT/M, CACHE/M.",
            theme,
        ),
        entry(
            "LAT p50",
            "median time-to-first-token over the last 30m, in ms.",
            theme,
        ),
        entry(
            "TP p50",
            "median output throughput over the last 30m, in tok/s.",
            theme,
        ),
        entry("UP 1d", "the endpoint's uptime over the last day.", theme),
        entry(
            "ECI \u{b1} CI",
            "the capability index with its bootstrap confidence interval.",
            theme,
        ),
        entry(
            "overrides",
            "conditional price rows above a minimum-prompt-token threshold \
             (the table's \u{2020} flag).",
            theme,
        ),
        note(
            "LAT and TP read \u{2014} without an OPENROUTER_API_KEY: the percentile \
             metrics are auth-gated.",
            theme,
        ),
    ]);
    lines
}

/// The Status line section: the freshness stamps and the degraded-mode
/// notes.
fn status_section(theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = section("Status line", theme);
    lines.extend([
        entry(
            "updated X ago",
            "freshness of the models list, the epoch data, and the endpoint detail — \
             rendered from cache stamps at draw time, never the network.",
            theme,
        ),
        entry(
            "mix",
            "the active cost-mix preset, with [m] adjust beside it.",
            theme,
        ),
        entry(
            "epoch: seed/downloaded/none",
            "where the ECI data came from: the snapshot shipped with jinn, a fresh \
             download, or nothing.",
            theme,
        ),
        note(
            "Loading and error notes appear here too — a failed fetch never blanks \
             the table.",
            theme,
        ),
    ]);
    lines
}

/// The Data sources section: who the performance data comes from, under
/// which license, with the attribution Epoch AI requires.
fn source_section(theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = section("Data sources", theme);
    lines.extend([
        entry(
            "Epoch AI (CC BY 4.0)",
            "performance data — the ECI and its confidence intervals. \
             https://epoch.ai/benchmarks/use-this-data",
            theme,
        ),
        note(
            "The seed CSVs compiled into jinn are Epoch's snapshot; <c-r> re-downloads \
             current data. Required attribution, verbatim:",
            theme,
        ),
        note(
            "Epoch AI, 'Capabilities & benchmarking'. Published online at epoch.ai. \
             Retrieved from 'https://epoch.ai/benchmarks/use-this-data' [online resource]. \
             Accessed 15 Oct 2026.",
            theme,
        ),
    ]);
    lines
}

/// The card's text: its title, then one section per thing the popup
/// puts on screen. Authored unwrapped; [`market_help_card`] wraps it to
/// the card's inner width.
fn help_lines(theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        "Model Market help",
        name_style(theme).add_modifier(Modifier::BOLD),
    ))];
    lines.extend(column_section(theme));
    lines.extend(key_section(theme));
    lines.extend(detail_section(theme));
    lines.extend(status_section(theme));
    lines.extend(source_section(theme));
    lines
}

/// The card's frame: the market popup's own accent around its text.
///
/// The border says *this box is about the model market* before a word
/// of it is read, and it keeps the card from bleeding into the table
/// behind it. The block carries the card's surface as well as its edge,
/// for the same reason every row restates the surface: a `Paragraph`
/// paints only the cells a line's text covers, and a card with a blank
/// row in it would otherwise show a hole in its own background rather
/// than a blank row of card.
fn help_card(theme: &Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(theme.popup_title)
                .bg(theme.infopopup_bg),
        )
        .style(Style::default().bg(theme.infopopup_bg))
}

/// Emits the current row and resets the fill: the new row's used width
/// is always 0.
fn break_row(rows: &mut Vec<Line<'static>>, current: &mut Vec<Span<'static>>) -> usize {
    if !current.is_empty() {
        rows.push(Line::from(std::mem::take(current)));
    }
    0
}

/// Wraps one authored line to `width` graphemes, breaking between words,
/// and returns the rows it occupies.
///
/// Rows break at unicode word boundaries; an authored gap serves the
/// break it causes — a run of whitespace that would start a row (or
/// overflow one) is dropped, not carried. A single word wider than a
/// row overflows its own row rather than being split mid-word. An empty
/// line stays exactly one row: it is the blank the author asked for.
fn wrap_line(line: Line<'static>, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    if line.spans.iter().all(|span| span.content.is_empty()) {
        return vec![line];
    }
    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    for span in line.spans {
        for word in span.content.split_word_bounds() {
            let is_gap = word.split_whitespace().next().is_none();
            let length = word.graphemes(true).count();
            if is_gap {
                // A run of whitespace: it keeps the words apart when it
                // fits, and becomes the row break when it does not.
                if used + length <= width {
                    current.push(Span::styled(word.to_owned(), span.style));
                    used += length;
                } else {
                    used = break_row(&mut rows, &mut current);
                }
                continue;
            }
            if used > 0 && used + length > width {
                used = break_row(&mut rows, &mut current);
            }
            current.push(Span::styled(word.to_owned(), span.style));
            used += length;
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

/// Where the card goes: its height, and the bounds it must fit in.
struct CardPlacement {
    /// The card's measured height.
    height: u16,
    /// One past the popup's last row, border included.
    popup_bottom: u16,
    /// The terminal's first row.
    terminal_top: u16,
    /// One past the terminal's last row.
    terminal_bottom: u16,
}

/// The card's top row: below the popup, flush with its border, or the
/// terminal's top row when the room below cannot hold the card.
///
/// Below is the card's only home — a card that flips sides as the
/// selection moves is a card that jumps under the reader. A terminal
/// too short to hold it draws it from the terminal's top row and lets
/// the last rows be cut: an overlay is laid over the screen, and there
/// is nowhere else to put it.
fn place_card(input: &CardPlacement) -> u16 {
    let below = input
        .popup_bottom
        .min(input.terminal_bottom.saturating_sub(input.height));
    if below.saturating_add(input.height) <= input.terminal_bottom {
        below
    } else {
        input.terminal_top
    }
}

/// The help card for the open popup, or `None` while it is hidden.
///
/// The card is drawn below the popup at the popup's inner width — the
/// table's own width — with the text wrapped to it. It is an overlay on
/// the terminal rather than a row inside the table, and it sizes itself
/// to its content, so its height is measured here, before it can be
/// placed.
#[must_use]
pub fn market_help_card(
    ui: &MarketUiState,
    popup_area: Rect,
    inner: Rect,
    terminal: Rect,
    theme: &Theme,
) -> Option<HelpCard> {
    if !ui.help_visible {
        return None;
    }
    let card = help_card(theme);
    // The text is drawn inside the border, so it wraps a card narrower
    // than the popup's inner width by the border's two cells. Wrapping
    // at the card's own width would under-count the rows, and a card
    // shorter than its text runs off its own bottom border.
    let text_width = inner.width.saturating_sub(CARD_BORDER_CELLS);
    let lines: Vec<Line<'static>> = help_lines(theme)
        .into_iter()
        .flat_map(|line| wrap_line(line, text_width))
        // `Line::style` replaces rather than patches, so the surface is
        // restated on every row: a body's own colors layered on a card
        // background would keep the foreground and drop the surface.
        .map(|line| line.style(Style::default().bg(theme.infopopup_bg)))
        .collect();
    // The card is a border around its text, so its height is the text's
    // plus the two border rows — measured here because the card is
    // placed before it is drawn.
    let height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2)
        .max(2);
    let y = place_card(&CardPlacement {
        height,
        // The card is kept off the popup *including its border*, not
        // just its body: a card over the popup's own edge reads as a
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

/// The help card, placed: the area it occupies, the text area inside
/// its border, the frame that draws the border, and the lines the text
/// is. The parts stay private — [`draw`](HelpCard::draw) is the only
/// thing that needs to know how they compose.
pub struct HelpCard {
    /// The card's whole area, border included.
    area: Rect,
    /// The area inside the border, where the text goes.
    text: Rect,
    /// The card's frame.
    card: Block<'static>,
    /// The card's text, already wrapped to `text`'s width.
    lines: Vec<Line<'static>>,
}

impl HelpCard {
    /// Draws the card: clear its area, lay the border, then the text
    /// inside it.
    ///
    /// The border goes down before the text so it paints the card's
    /// surface; the text then goes into the area the border leaves.
    /// Doing this here rather than at the call site keeps the card's
    /// parts private — a caller that reached for `area` and `card`
    /// separately would have to know the order they compose in.
    pub fn draw(self, frame: &mut Frame<'_>) {
        frame.render_widget(Clear, self.area);
        frame.render_widget(self.card, self.area);
        frame.render_widget(Paragraph::new(self.lines), self.text);
    }
}
