//! Authorization: decide whether an inbound Discord actor may interact.
//!
//! Deny-by-default identity gate for the Discord frontend. Everything a
//! Discord user sends — plain messages and every slash command — flows
//! through [`is_authorized`] before any state is touched or any bus command
//! is published. The allow-list is `[discord].authorized_users` in
//! `jinn.toml`: numeric user IDs (as strings). An empty or missing list
//! authorizes nobody, and entries that don't parse as numeric IDs can never
//! match, so malformed config can only ever tighten the gate.

/// Whether `author_id` may interact with the bot.
///
/// Authorized iff at least one configured entry equals `author_id` after
/// trimming surrounding whitespace and parsing as a numeric ID; unparsable
/// entries never match.
#[must_use]
pub fn is_authorized(authorized_users: &[String], author_id: u64) -> bool {
    authorized_users.iter().any(|raw| {
        raw.trim()
            .parse::<u64>()
            .is_ok_and(|allowed| allowed == author_id)
    })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::is_authorized;

    #[rstest]
    #[case::listed_id("123456789012345678", 123_456_789_012_345_678)]
    #[case::padded_entry("  123456789012345678\t", 123_456_789_012_345_678)]
    fn allows_configured_author(#[case] entry: &str, #[case] author_id: u64) {
        // Given an allow-list containing the author's id
        //     (possibly with surrounding whitespace).
        let authorized_users = [entry.to_owned()];

        // When checking authorization.
        let allowed = is_authorized(&authorized_users, author_id);

        // Then the author is authorized.
        assert!(allowed);
    }

    #[test]
    fn rejects_unlisted_author() {
        // Given an allow-list that does not contain the author's id.
        let authorized_users = &["123456789012345678".to_owned()];

        // When checking authorization.
        let allowed = is_authorized(authorized_users, 876_543_210_987_654_321);

        // Then the author is denied.
        assert!(!allowed);
    }

    #[test]
    fn denies_everyone_when_list_empty() {
        // Given an empty allow-list (also what a missing key deserializes to).
        let authorized_users: &[String] = &[];

        // When checking authorization.
        let allowed = is_authorized(authorized_users, 123_456_789_012_345_678);

        // Then nobody is authorized.
        assert!(!allowed);
    }

    #[test]
    fn malformed_entries_dont_block_valid_ones() {
        // Given an allow-list mixing a garbled entry with the author's id.
        let authorized_users = &[
            "not-a-snowflake".to_owned(),
            "123456789012345678".to_owned(),
        ];

        // When checking authorization.
        let allowed = is_authorized(authorized_users, 123_456_789_012_345_678);

        // Then the author is authorized via the valid entry.
        assert!(allowed);
    }

    #[test]
    fn zero_author_id_matches_only_zero_entry() {
        // Given an allow-list containing "0".
        let authorized_users = &["0".to_owned()];

        // When checking id 0 against it.
        let matched = is_authorized(authorized_users, 0);

        // Then it matches (and would never collide with a real snowflake).
        assert!(matched);
    }

    #[test]
    fn near_collide_ids_are_distinguished() {
        // Given an allow-list holding one specific snowflake.
        let authorized_users = &["123456789012345678".to_owned()];

        // When checking ids that differ from it by one digit
        //     (leading and trailing).
        let leading = is_authorized(authorized_users, 223_456_789_012_345_678);
        let trailing = is_authorized(authorized_users, 123_456_789_012_345_679);

        // Then neither near-collision is authorized.
        assert!(!leading);
        assert!(!trailing);
    }
}
