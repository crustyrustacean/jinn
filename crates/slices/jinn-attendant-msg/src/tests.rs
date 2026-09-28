#[cfg(test)]
mod attendant_msg_tests {
    #![allow(clippy::unwrap_used, reason = "test code")]

    use jiff::Timestamp;

    use crate::{AttendantActivation, AttendantReport, AttendantTrigger, PRIOR_REPORT_PLACEHOLDER};

    #[rstest::rstest]
    fn activation_defaults_to_seed() {
        // Given no explicit activation.

        // When creating the default activation.
        let activation = AttendantActivation::default();

        // Then the attendant is still being composed.
        assert_eq!(activation, AttendantActivation::Seed);
    }

    #[rstest::rstest]
    fn trigger_defaults_to_manual() {
        // Given no explicit trigger.

        // When creating the default trigger.
        let trigger = AttendantTrigger::default();

        // Then the attendant only runs when the user asks.
        assert_eq!(trigger, AttendantTrigger::Manual);
    }

    #[rstest::rstest]
    #[case(AttendantActivation::Seed, false)]
    #[case(AttendantActivation::Reset, true)]
    #[case(AttendantActivation::Continue, true)]
    fn seed_activation_is_not_dispatchable(
        #[case] activation: AttendantActivation,
        #[case] expected: bool,
    ) {
        // Given an activation mode.

        // When checking whether a run may dispatch.
        let dispatchable = activation.is_dispatchable();

        // Then only the composing mode is blocked.
        assert_eq!(dispatchable, expected);
    }

    #[rstest::rstest]
    fn default_seed_template_carries_the_placeholder() {
        // Given no user-written seed template.

        // When reading the default template.
        let template = crate::default_seed_template();

        // Then it refers to the prior report by placeholder.
        assert!(template.contains(PRIOR_REPORT_PLACEHOLDER));
    }

    #[rstest::rstest]
    fn report_round_trips_through_json() {
        // Given a published report.
        let report = AttendantReport {
            run: 2,
            published_at: Timestamp::from_second(1_700_000_000).unwrap(),
            body: "the migration is missing a WHERE clause".to_owned(),
        };

        // When it survives a serialization round trip.
        let json = serde_json::to_string(&report).unwrap();
        let restored: AttendantReport = serde_json::from_str(&json).unwrap();

        // Then every field is preserved.
        assert_eq!(restored, report);
    }
}
