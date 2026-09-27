//! Shared `min_age` helper for auto-prune workers.
//!
//! Each worker that supports a `min_age` floor uses [`is_within_min_age`] to
//! decide whether a candidate entry is too recent to prune. The age of an entry
//! is the number of entries between it and the end of history:
//!
//! `age = history_len - (entry_idx + 1)`
//!
//! The last entry in history has age 0; an entry 50 slots back from the end of
//! a 100-entry history has age 49.
//!
//! # Protection rule
//!
//! An entry is **protected** when `age < min_age`. With `min_age = 0`, no entry
//! is ever protected (back-compat with workers that previously had no floor).
//!
//! Out-of-range indices (`entry_idx >= history_len`) return `false` (not
//! protected). In practice workers only pass indices they obtained from
//! iterating history, so this branch is purely defensive.

/// Returns `true` if the entry at `entry_idx` is within `min_age` entries of
/// the end of a history of length `history_len`. Uses checked subtraction so
/// an out-of-range index returns `false` (i.e., not protected).
#[must_use]
pub(crate) const fn is_within_min_age(
    history_len: usize,
    entry_idx: usize,
    min_age: usize,
) -> bool {
    // `entry_idx + 1` first; if that overflows, treat as out-of-range.
    let Some(offset) = entry_idx.checked_add(1) else {
        return false;
    };
    let Some(age) = history_len.checked_sub(offset) else {
        return false;
    };
    age < min_age
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unreachable,
        clippy::string_slice,
        clippy::uninlined_format_args,
        reason = "test code"
    )]
    use super::is_within_min_age;

    // age = history_len - entry_idx - 1
    //   last entry (idx = history_len - 1): age = 0
    //   50 back in a 100-len history (idx = 50): age = 49

    #[rstest::rstest]
    #[test]
    fn age_zero_protected() {
        // Given the last entry of a 100-entry history with min_age = 1.
        let history_len = 100;
        let entry_idx = 99;
        let min_age = 1;

        // When checking whether the entry is protected.
        let protected = is_within_min_age(history_len, entry_idx, min_age);

        // Then it is protected, because age 0 < 1.
        assert!(protected);
    }

    #[rstest::rstest]
    #[test]
    fn age_at_boundary_protected() {
        // Given entry 50 of a 100-entry history with min_age = 50.
        let history_len = 100;
        let entry_idx = 50;
        let min_age = 50;

        // When checking whether the entry is protected.
        let protected = is_within_min_age(history_len, entry_idx, min_age);

        // Then it is protected, because age 49 is strictly less than 50.
        assert!(protected);
    }

    #[rstest::rstest]
    #[test]
    fn age_at_boundary_not_protected() {
        // Given entry 49 of a 100-entry history with min_age = 50.
        let history_len = 100;
        let entry_idx = 49;
        let min_age = 50;

        // When checking whether the entry is protected.
        let protected = is_within_min_age(history_len, entry_idx, min_age);

        // Then it is not protected, because age 50 is not strictly less than 50.
        assert!(!protected);
    }

    #[rstest::rstest]
    #[test]
    fn min_age_zero_never_protects() {
        // Given histories of length 100, 100, and 1, all with min_age = 0.
        let histories = [(100, 99), (100, 0), (1, 0)];

        // When checking each entry for protection.
        let protected: Vec<bool> = histories
            .iter()
            .map(|(history_len, entry_idx)| is_within_min_age(*history_len, *entry_idx, 0))
            .collect();

        // Then none of them is protected.
        assert_eq!(protected, vec![false, false, false]);
    }

    #[rstest::rstest]
    #[test]
    fn out_of_range_index_not_protected() {
        // Given out-of-range indices, all with min_age = 5.
        let cases = [(10, 10), (10, 100), (0, 0), (0, usize::MAX)];

        // When checking each for protection.
        let protected: Vec<bool> = cases
            .iter()
            .map(|(history_len, entry_idx)| is_within_min_age(*history_len, *entry_idx, 5))
            .collect();

        // Then none of them is protected.
        assert_eq!(protected, vec![false, false, false, false]);
    }
}
