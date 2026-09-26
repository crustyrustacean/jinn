//! Spinner pacing and phase selection, shared by every loading indicator.
//!
//! Spinners are drawn in four places — the chat log's session-load line, the
//! provider's streaming indicator, the sidebar's per-session working marker,
//! and the session preview popup — and each one has to agree on how fast the
//! animation turns, or they visibly beat against one another. The interval
//! lives here so there is one answer to that question.
//!
//! What does *not* live here is the animation *state*. The three established
//! spinners draw through `Throbber::render_stateful_widget`, which keeps the
//! index itself; only the preview popup needs a bare glyph, and it has nowhere
//! to keep a widget state. [`spinner_glyph`] serves that case by deriving the
//! phase from elapsed time instead.

use std::time::Duration;

/// Minimum time between spinner frame advances.
pub const SPINNER_INTERVAL: Duration = Duration::from_millis(80);

/// Which symbol of a set a spinner of `len` symbols should show at `elapsed`.
///
/// Derived from elapsed time rather than an accumulated step count, so a spinner
/// with no stored state still turns, and two spinners started together stay in
/// step. `len` is clamped to at least one symbol, so an empty set cannot
/// divide by zero.
///
/// The phase is offset by one: at zero elapsed a throbber that has just
/// advanced once already sits on symbol 1, and a spinner frozen on symbol 0
/// reads as "not running". Advancing past the wrap would be wrong, so the
/// offset is applied *before* the modulo.
#[must_use]
pub fn spinner_index(elapsed: Duration, len: usize) -> usize {
    let symbols = len.max(1);
    let step = u32::try_from(elapsed.as_millis() / SPINNER_INTERVAL.as_millis()).unwrap_or(0);
    // `step + 1` can only overflow at ~4.3 billion steps, which is a spinner
    // running for a decade; wrapping to 0 is a harmless last resort.
    usize::try_from((step + 1) % symbols as u32).unwrap_or(0)
}

/// The ASCII spinner glyph to show at `elapsed`.
///
/// The preview popup's spinner: a single [`Span`] in a plain `Paragraph`, with
/// no widget to hold a [`throbber_widgets_tui::ThrobberState`].
#[must_use]
#[expect(
    clippy::indexing_slicing,
    reason = "spinner_index mods by the symbol count it was just given"
)]
pub fn spinner_glyph(elapsed: Duration) -> &'static str {
    throbber_widgets_tui::ASCII.symbols
        [spinner_index(elapsed, throbber_widgets_tui::ASCII.symbols.len())]
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::*;

    /// The symbols of a set of `len`, so a case can assert against a known ring.
    fn ring(len: usize) -> Vec<usize> {
        (0..len).collect()
    }

    #[rstest::rstest]
    #[case::start(0, 1)]
    #[case::one_step(1, 2)]
    #[case::two_steps(2, 3)]
    #[case::wraps_at_len(4, 1)]
    #[case::wraps_again(5, 2)]
    fn spinner_index_advances_with_elapsed_time(#[case] steps: u32, #[case] expected: usize) {
        // Given a spinner set of four symbols, some elapsed time.
        let elapsed = SPINNER_INTERVAL * steps;

        // When the phase is derived from that elapsed time.
        let index = spinner_index(elapsed, 4);

        // Then it advanced one step per interval, wrapping at the set size.
        assert_eq!(
            index, expected,
            "expected step {steps} to land on {expected}"
        );
    }

    #[rstest::rstest]
    fn spinner_index_wraps_within_a_short_set() {
        // Given a set of two symbols, several steps of elapsed time.
        let elapsed = SPINNER_INTERVAL * 5;

        // When the phase is derived.
        let index = spinner_index(elapsed, 2);

        // Then it is a valid index for that set — the modulo, not a truncated step.
        assert!(
            ring(2).contains(&index),
            "index {index} is outside a two-symbol set"
        );
    }

    #[rstest::rstest]
    fn spinner_index_survives_an_empty_symbol_set() {
        // Given a spinner set with no symbols at all.
        let len = 0;

        // When the phase is derived.
        let index = spinner_index(SPINNER_INTERVAL, len);

        // Then it still yields a usable index rather than dividing by zero.
        assert_eq!(index, 0, "an empty set is clamped to one symbol");
    }

    #[rstest::rstest]
    fn spinner_glyph_comes_from_the_ascii_set_after_a_very_long_time() {
        // Given a spinner that has been running for hours.
        let elapsed = Duration::from_hours(100);

        // When the glyph is derived.
        let glyph = spinner_glyph(elapsed);

        // Then it is one of the ASCII spinner's symbols — the wrap held.
        assert!(
            throbber_widgets_tui::ASCII.symbols.contains(&glyph),
            "glyph {glyph:?} is not in the ASCII spinner set"
        );
    }

    #[rstest::rstest]
    fn spinner_glyph_changes_over_time() {
        // Given the same spinner one interval apart.
        let first = spinner_glyph(Duration::ZERO);
        let second = spinner_glyph(SPINNER_INTERVAL);

        // Then the glyph advanced — the animation is actually turning.
        assert_ne!(
            first, second,
            "a spinner frozen on one glyph does not read as running"
        );
    }
}
