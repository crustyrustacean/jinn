//! Wall-clock intervals describing when a session was working.
//!
//! A session is working whenever its phase is not idle — including while it
//! waits on its own tools or on its subagents. That span cannot be
//! reconstructed from per-entry timings: the gap between one assistant entry
//! finishing and the next dispatching contains the whole tool loop, and an
//! interrupted session leaves no finish timestamp at all.
//!
//! So the time is recorded directly, as a list of [`WorkingInterval`]s rather
//! than a running total. An interval carries the moment work started, the
//! moment it stopped, and whether it is still open — the third fact is what
//! makes a session killed mid-turn bill only up to the moment it stopped
//! rather than up to whenever it was next loaded.
//!
//! Intervals are stored coalesced (see [`coalesce`]) so one continuous stretch
//! of work reads as one interval even when its transitions were observed in
//! pieces.

use std::cmp::Ordering;

use jiff::SignedDuration;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// The default largest gap that still counts as continuous work.
///
/// Two working intervals separated by no more than this are merged into one,
/// so a turn that hands off through an intermediate phase does not render as
/// several disjoint slivers. A gap above it is real downtime and stays a
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoalescingGap(jiff::SignedDuration);

impl CoalescingGap {
    /// The coalescing threshold, two seconds.
    #[must_use]
    pub const fn two_seconds() -> Self {
        Self(jiff::SignedDuration::from_secs(2))
    }

    /// Whether a gap of `duration` is close enough to merge across.
    ///
    /// Compared in nanoseconds rather than whole seconds so the threshold is
    /// exactly 2s: a 2s gap merges and a 2s + 1ns gap does not. Comparing
    /// truncated seconds would admit everything up to 2.999s.
    #[must_use]
    pub const fn admits(self, duration: &SignedDuration) -> bool {
        !duration.is_negative() && duration.as_nanos() <= self.0.as_nanos()
    }
}

/// One closed or open stretch of wall-clock time during which work happened.
///
/// `end: None` means the interval is still open: work started and has not
/// stopped. An open interval bills time only up to the moment it is closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingInterval {
    /// When work started.
    start: Timestamp,
    /// When work stopped, or `None` while it is still running.
    end: Option<Timestamp>,
}

impl WorkingInterval {
    /// An interval that ran from `start` to `end`.
    #[must_use]
    pub const fn closed(start: Timestamp, end: Timestamp) -> Self {
        Self {
            start,
            end: Some(end),
        }
    }

    /// An interval that began at `start` and has not stopped.
    #[must_use]
    pub const fn open(start: Timestamp) -> Self {
        Self { start, end: None }
    }

    /// When this interval began.
    #[must_use]
    pub const fn start(&self) -> Timestamp {
        self.start
    }

    /// When this interval stopped, or `None` while it is still open.
    #[must_use]
    pub const fn end(&self) -> Option<Timestamp> {
        self.end
    }

    /// Whether this interval is still running.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.end.is_none()
    }

    /// Closes this interval at `at`, if it was open.
    ///
    /// Returns `true` when this call closed it. A no-op on an already-closed
    /// interval — a forced `Idle → Idle` publish can otherwise double-close
    /// the boundary and produce a zero-length span.
    pub fn close(&mut self, at: Timestamp) -> bool {
        if self.end.is_some() {
            return false;
        }
        self.end = Some(at);
        true
    }

    /// The wall-clock gap between this interval and `next`, or `None` when
    /// they overlap or when this interval is still open.
    ///
    /// An open interval has no end to measure from, so it reports `None` and
    /// the caller treats it as contiguous with whatever follows.
    #[must_use]
    pub fn gap_to(&self, next: &Self) -> Option<SignedDuration> {
        let end = self.end?;
        let gap = nanos_between(end, next.start);
        if gap.is_negative() { None } else { Some(gap) }
    }

    /// How long this interval ran, measured to `at` if it is still open.
    ///
    /// A negative span (a wall-clock step backwards) clamps to zero rather
    /// than subtracting from a total.
    #[must_use]
    pub fn duration_until(&self, at: Timestamp) -> SignedDuration {
        let span = self.signed_span_to(self.end.unwrap_or(at));
        if span.is_negative() {
            SignedDuration::ZERO
        } else {
            span
        }
    }

    /// The signed span from this interval's start to `end`, sign preserved.
    fn signed_span_to(&self, end: Timestamp) -> SignedDuration {
        nanos_between(self.start, end)
    }
}

/// The signed nanosecond distance from `start` to `end`.
///
/// Saturated and clamped into `i64` because [`SignedDuration::from_nanos`]
/// takes an `i64`; a timestamp pair this far apart is a clock error, not a
/// duration worth billing.
fn nanos_between(start: Timestamp, end: Timestamp) -> SignedDuration {
    let nanos = end
        .as_nanosecond()
        .saturating_sub(start.as_nanosecond())
        .clamp(i128::from(i64::MIN), i128::from(i64::MAX));
    SignedDuration::from_nanos(nanos as i64)
}

/// Sorts `intervals` by start and merges every pair separated by no more than
/// `gap`.
///
/// The result is disjoint and ordered. A merged interval stays open if any of
/// its inputs was open: work that has not stopped must not acquire an end from
/// a neighbour that stopped, or a running session bills only as far as the
/// last closed neighbour.
pub fn coalesce(intervals: &mut Vec<WorkingInterval>, gap: CoalescingGap) {
    if intervals.len() < 2 {
        return;
    }
    intervals.sort_by(|a, b| match a.start.cmp(&b.start) {
        Ordering::Equal => a.end.cmp(&b.end),
        other => other,
    });

    let mut merged: Vec<WorkingInterval> = Vec::with_capacity(intervals.len());
    for interval in intervals.drain(..) {
        match merged.last_mut() {
            Some(last) => {
                let abuts = match last.gap_to(&interval) {
                    Some(distance) => gap.admits(&distance),
                    // Overlapping, or the left side is still running.
                    None => true,
                };
                if !abuts {
                    merged.push(interval);
                    continue;
                }
                // Still running wins: an open left side keeps its `None` end so
                // the merged interval stays open. Otherwise the later of the two
                // ends wins, so a closed interval always extends a shorter one.
                last.end = match (last.end, interval.end) {
                    (None, _) | (_, None) => None,
                    (Some(earlier), Some(later)) => Some(earlier.max(later)),
                };
            }
            None => merged.push(interval),
        }
    }
    *intervals = merged;
}

/// Total working time across `intervals`, measured to `at` for open ones.
///
/// Assumes the list is already coalesced; call [`coalesce`] first. Summing a
/// non-disjoint list double-counts any shared second.
#[must_use]
pub fn total(intervals: &[WorkingInterval], at: Timestamp) -> SignedDuration {
    intervals
        .iter()
        .fold(SignedDuration::ZERO, |sum, interval| {
            sum + interval.duration_until(at)
        })
}

/// Wall-clock working time of every interval in `lists` combined, counting a
/// second shared by two lists once.
///
/// This is what a session tree wants: a parent blocked on two subagents is
/// itself non-idle, so summing per-session totals would bill the same second
/// three times.
#[must_use]
pub fn union(lists: &[&[WorkingInterval]], at: Timestamp) -> SignedDuration {
    let mut all: Vec<WorkingInterval> =
        lists.iter().flat_map(|list| list.iter().cloned()).collect();
    coalesce(&mut all, CoalescingGap::two_seconds());
    total(&all, at)
}

/// Renders a duration as zero-padded `HH:MM:SS`, never abbreviated.
///
/// Hours accumulate past 24, so a long-running tree reads `71:00:00` rather
/// than rolling into days the user never sees. A negative duration renders as
/// zero.
#[must_use]
pub fn format_working_duration(duration: &SignedDuration) -> String {
    let total_seconds = match u64::try_from(duration.as_secs()) {
        Ok(seconds) => seconds,
        // A negative duration is a wall-clock step, not a negative duration.
        Err(_) => 0,
    };
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
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

    const START: i64 = 1_000_000_000;

    fn at(offset_secs: i64) -> Timestamp {
        Timestamp::from_second(START + offset_secs).expect("valid offset")
    }

    fn nanos(offset_nanos: i64) -> Timestamp {
        Timestamp::from_second(START)
            .expect("valid start")
            .checked_add(SignedDuration::from_nanos(offset_nanos))
            .expect("no overflow")
    }

    #[rstest::rstest]
    fn close_marks_the_interval_closed() {
        // Given an open interval.
        let mut interval = WorkingInterval::open(at(0));

        // When closing it.
        let closed = interval.close(at(5));

        // Then it is closed at the given moment.
        assert!(closed);
        assert!(!interval.is_open());
        assert_eq!(interval.end(), Some(at(5)));
    }

    #[rstest::rstest]
    fn closing_an_already_closed_interval_is_a_no_op() {
        // Given an interval closed at 5.
        let mut interval = WorkingInterval::closed(at(0), at(5));

        // When closing it again at 9.
        let closed = interval.close(at(9));

        // Then the close reports no change and the end stands.
        assert!(!closed);
        assert_eq!(interval.end(), Some(at(5)));
    }

    #[rstest::rstest]
    fn intervals_a_gap_of_exactly_the_threshold_are_merged() {
        // Given two intervals separated by exactly the two-second threshold.
        let mut intervals = vec![
            WorkingInterval::closed(at(0), at(3)),
            WorkingInterval::closed(at(5), at(8)),
        ];

        // When coalescing.
        coalesce(&mut intervals, CoalescingGap::two_seconds());

        // Then they are one interval spanning both.
        assert_eq!(intervals.len(), 1, "{intervals:?}");
        assert_eq!(intervals[0].start(), at(0));
        assert_eq!(intervals[0].end(), Some(at(8)));
    }

    #[rstest::rstest]
    fn intervals_a_nanosecond_past_the_threshold_stay_separate() {
        // Given two intervals separated by 2s + 1ns.
        let mut intervals = vec![
            WorkingInterval::closed(at(0), at(3)),
            WorkingInterval::closed(nanos(5_000_000_001), at(8)),
        ];

        // When coalescing.
        coalesce(&mut intervals, CoalescingGap::two_seconds());

        // Then neither is absorbed by the other.
        assert_eq!(intervals.len(), 2, "{intervals:?}");
    }

    #[rstest::rstest]
    fn overlapping_intervals_are_merged() {
        // Given two intervals that share a second.
        let mut intervals = vec![
            WorkingInterval::closed(at(0), at(10)),
            WorkingInterval::closed(at(5), at(15)),
        ];

        // When coalescing.
        coalesce(&mut intervals, CoalescingGap::two_seconds());

        // Then one interval spans the union.
        assert_eq!(intervals.len(), 1, "{intervals:?}");
        assert_eq!(intervals[0].end(), Some(at(15)));
    }

    #[rstest::rstest]
    fn coalescing_an_open_interval_against_a_closed_one_stays_open() {
        // Given an open interval followed by a closed one that starts inside it.
        let mut intervals = vec![
            WorkingInterval::open(at(0)),
            WorkingInterval::closed(at(4), at(9)),
        ];

        // When coalescing.
        coalesce(&mut intervals, CoalescingGap::two_seconds());

        // Then the merged interval is still open, so a running session is not
        // billed only as far as its neighbour's end.
        assert_eq!(intervals.len(), 1, "{intervals:?}");
        assert!(intervals[0].is_open(), "{intervals:?}");
    }

    #[rstest::rstest]
    fn total_sums_the_lengths_of_a_coalesced_list() {
        // Given two disjoint intervals.
        let intervals = vec![
            WorkingInterval::closed(at(0), at(10)),
            WorkingInterval::closed(at(20), at(23)),
        ];

        // When totalling them.
        let total = total(&intervals, at(100));

        // Then the total is 13 seconds.
        assert_eq!(total, SignedDuration::from_secs(13));
    }

    #[rstest::rstest]
    fn union_counts_a_shared_second_once() {
        // Given a parent working 0..10 and a child working 4..14 entirely
        // inside it.
        let parent = vec![WorkingInterval::closed(at(0), at(10))];
        let child = vec![WorkingInterval::closed(at(4), at(14))];

        // When unioning them.
        let working = union(&[&parent, &child], at(100));

        // Then the result is the 14-second span, not 10 + 10.
        assert_eq!(working, SignedDuration::from_secs(14));
    }

    #[rstest::rstest]
    fn union_measures_an_open_interval_to_the_given_moment() {
        // Given a parent still working and a finished child.
        let parent = vec![WorkingInterval::open(at(0))];
        let child = vec![WorkingInterval::closed(at(4), at(9))];

        // When unioning at 10 seconds.
        let working = union(&[&parent, &child], at(10));

        // Then the open interval bills only up to 10.
        assert_eq!(working, SignedDuration::from_secs(10));
    }

    #[rstest::rstest]
    fn union_of_nothing_is_zero() {
        // Given no intervals at all.
        // When unioning.
        let working = union(&[], at(10));
        // Then nothing was worked.
        assert_eq!(working, SignedDuration::ZERO);
    }

    #[rstest::rstest]
    #[case(0, "00:00:00")]
    #[case(9, "00:00:09")]
    #[case(59, "00:00:59")]
    #[case(60, "00:01:00")]
    #[case(600, "00:10:00")]
    #[case(3600, "01:00:00")]
    // Hours accumulate past a day rather than rolling into a days field.
    #[case(71 * 3600, "71:00:00")]
    fn format_renders_zero_padded_hms(#[case] seconds: i64, #[case] expected: &str) {
        // Given a duration of whole seconds.
        let duration = SignedDuration::from_secs(seconds);

        // When formatting it.
        let rendered = format_working_duration(&duration);

        // Then it reads as zero-padded HH:MM:SS.
        assert_eq!(rendered, expected);
    }

    #[rstest::rstest]
    fn format_renders_a_negative_duration_as_zero() {
        // Given a duration before a wall-clock step.
        let duration = SignedDuration::from_secs(-5);

        // When formatting it.
        let rendered = format_working_duration(&duration);

        // Then the display does not go negative.
        assert_eq!(rendered, "00:00:00");
    }
}
