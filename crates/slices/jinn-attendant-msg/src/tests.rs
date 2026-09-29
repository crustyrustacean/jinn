#[cfg(test)]
mod attendant_msg_tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, reason = "test code")]

    use jiff::Timestamp;

    use crate::{
        AttendantActivation, AttendantContextPolicy, AttendantReport, AttendantTrigger,
        PRIOR_REPORT_PLACEHOLDER,
    };
    #[rstest::rstest]
    fn the_preserve_mode_is_named_preserve_on_disk() {
        // Given the mode that keeps the prior conversation intact.
        let mode = AttendantActivation::Preserve;

        // When it is serialized as stored in session metadata.
        let json = serde_json::to_string(&mode).expect("serializes");

        // Then the stored name is `preserve`. The name has to be this, not
        // `continue`: the `c` key already means "continue this session", and
        // one word carrying two meanings is how a mode gets misread.
        assert_eq!(json, r#""preserve""#);
        // And it round-trips back to the same mode.
        assert_eq!(
            serde_json::from_str::<AttendantActivation>(&json).expect("deserializes"),
            mode
        );
    }

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
    #[case(AttendantActivation::Preserve, true)]
    fn a_parent_completed_trigger_is_enabled_for_every_mode_but_seed(
        #[case] activation: AttendantActivation,
        #[case] expected: bool,
    ) {
        // Given an activation mode.

        // When checking whether a parent-completed fire is configured.
        let enabled = AttendantTrigger::ParentCompleted.is_enabled_for(activation);

        // Then only the composing mode is blocked. Preserve is as runnable
        // as Reset — it is a mode, not a veto.
        assert_eq!(enabled, expected);
    }

    #[rstest::rstest]
    #[case(AttendantActivation::Seed, AttendantContextPolicy::Pin)]
    #[case(AttendantActivation::Reset, AttendantContextPolicy::Reset)]
    #[case(AttendantActivation::Preserve, AttendantContextPolicy::Preserve)]
    fn each_activation_mode_answers_exactly_what_it_does_to_context(
        #[case] activation: AttendantActivation,
        #[case] expected: AttendantContextPolicy,
    ) {
        // Given an activation mode.

        // When asking how it prepares context.
        let policy = activation.context_policy();

        // Then the mode answers only that, and every mode has an answer.
        assert_eq!(policy, expected);
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

#[cfg(test)]
mod properties_tests {
    use crate::{
        ACTIVATION_CHOICES, AttendantActivation, AttendantPropertiesState, AttendantTrigger,
        OriginalValues, PickDirection, PropertyField, TRIGGER_CHOICES, attendant_properties_scope,
        attendant_seed_template_scope, pick_activation, pick_trigger,
    };

    #[rstest::rstest]
    fn form_focus_defaults_to_the_first_field() {
        // Given a fresh popup state.

        // When reading its field focus.
        let focus = AttendantPropertiesState::default().focus;

        // Then the cursor rests on the trigger, the first field in display
        // order. Field movement clamps, so a default on the last field would
        // be a dead end — `j` could not leave it.
        assert_eq!(focus, PropertyField::Trigger);
    }

    #[rstest::rstest]
    fn trigger_choices_are_ordered_parent_completed_then_manual() {
        // Given the trigger choice row.

        // When reading its values in display order.
        let values: Vec<_> = TRIGGER_CHOICES.iter().map(|(v, _)| *v).collect();

        // Then parent-completed is leftmost and manual is rightmost.
        assert_eq!(
            values,
            vec![AttendantTrigger::ParentCompleted, AttendantTrigger::Manual]
        );
    }

    #[rstest::rstest]
    fn activation_choices_are_ordered_seed_reset_preserve() {
        // Given the activation choice row.

        // When reading its values in display order.
        let values: Vec<_> = ACTIVATION_CHOICES.iter().map(|(v, _)| *v).collect();

        // Then the row reads seed, reset, preserve.
        assert_eq!(
            values,
            vec![
                AttendantActivation::Seed,
                AttendantActivation::Reset,
                AttendantActivation::Preserve
            ]
        );
    }

    #[rstest::rstest]
    fn picking_left_from_the_first_choice_clamps() {
        // Given the leftmost trigger choice.

        // When picking left.
        let picked = pick_trigger(AttendantTrigger::ParentCompleted, PickDirection::Left);

        // Then the choice does not move.
        assert_eq!(picked, AttendantTrigger::ParentCompleted);
    }

    #[rstest::rstest]
    fn picking_right_from_the_last_choice_clamps() {
        // Given the rightmost activation choice.

        // When picking right.
        let picked = pick_activation(AttendantActivation::Preserve, PickDirection::Right);

        // Then the choice does not move.
        assert_eq!(picked, AttendantActivation::Preserve);
    }

    #[rstest::rstest]
    fn picking_moves_one_choice_per_key() {
        // Given a mid-row activation choice.

        // When picking in each direction.
        let left = pick_activation(AttendantActivation::Reset, PickDirection::Left);
        let right = pick_activation(AttendantActivation::Reset, PickDirection::Right);

        // Then each pick lands on the adjacent choice.
        assert_eq!(left, AttendantActivation::Seed);
        assert_eq!(right, AttendantActivation::Preserve);
    }

    #[rstest::rstest]
    fn properties_scope_is_navigation_only() {
        // Given the properties popup's scope.

        // When checking whether it captures text input.
        let captures = attendant_properties_scope().captures_input();

        // Then it does not: typed characters never land on the form.
        assert!(!captures);
    }

    #[rstest::rstest]
    fn seed_template_scope_captures_input() {
        // Given the seed-template editor's scope.

        // When checking whether it captures text input.
        let captures = attendant_seed_template_scope().captures_input();

        // Then it does: the editor needs the editing keys and the
        // printable catch-all.
        assert!(captures);
    }

    #[rstest::rstest]
    fn field_movement_stops_at_the_last_field() {
        // Given a popup whose cursor is on the last field.

        // When moving forward.
        let mut popup = AttendantPropertiesState {
            focus: PropertyField::SeedTemplate,
            ..AttendantPropertiesState::default()
        };
        popup.focus_next();

        // Then the cursor stays put. Wrapping would hide the boundary — a
        // user pressing `j` at the bottom of a form needs to know they are
        // at the bottom, and the choices here already clamp at their ends.
        assert_eq!(popup.focus, PropertyField::SeedTemplate);
    }

    #[rstest::rstest]
    fn field_movement_stops_at_the_first_field() {
        // Given a popup whose cursor is on the first field.

        // When moving backward.
        let mut popup = AttendantPropertiesState {
            focus: PropertyField::Trigger,
            ..AttendantPropertiesState::default()
        };
        popup.focus_previous();

        // Then the cursor stays put.
        assert_eq!(popup.focus, PropertyField::Trigger);
    }

    #[rstest::rstest]
    fn every_field_is_reachable_by_moving_down_from_the_first() {
        // Given a popup opened on its first field.

        // When moving down once per press, three times.
        let mut popup = AttendantPropertiesState::default();
        let mut visited = vec![popup.focus];
        for _ in 0..2 {
            popup.focus_next();
            visited.push(popup.focus);
        }

        // Then all three fields were visited in display order.
        assert_eq!(
            visited,
            vec![
                PropertyField::Trigger,
                PropertyField::Activation,
                PropertyField::SeedTemplate,
            ]
        );
    }

    #[rstest::rstest]
    fn pick_moves_the_focused_choice_row_only() {
        // Given a popup focused on the trigger with pending edits on both
        // rows.
        let mut popup = AttendantPropertiesState {
            focus: PropertyField::Trigger,
            pending_activation: AttendantActivation::Seed,
            pending_trigger: AttendantTrigger::ParentCompleted,
            ..AttendantPropertiesState::default()
        };

        // When picking left.
        popup.pick(PickDirection::Left);

        // Then the trigger moved to its leftmost choice.
        assert_eq!(popup.pending_trigger, AttendantTrigger::ParentCompleted);
        // And the activation did not move.
        assert_eq!(popup.pending_activation, AttendantActivation::Seed);

        // Given the same popup refocused on the activation.
        popup.focus = PropertyField::Activation;

        // When picking right.
        popup.pick(PickDirection::Right);

        // Then the activation moved one choice right.
        assert_eq!(popup.pending_activation, AttendantActivation::Reset);
        // And the trigger did not move.
        assert_eq!(popup.pending_trigger, AttendantTrigger::ParentCompleted);
    }

    #[rstest::rstest]
    fn pick_is_a_noop_on_the_seed_template_field() {
        // Given a popup focused on the seed template.
        let mut popup = AttendantPropertiesState::default();

        // When picking in either direction.
        popup.pick(PickDirection::Left);
        popup.pick(PickDirection::Right);

        // Then nothing changed: the template is not a choice row.
        assert_eq!(popup.pending_trigger, AttendantTrigger::Manual);
        assert_eq!(popup.pending_activation, AttendantActivation::Seed);
    }

    #[rstest::rstest]
    fn restore_original_restores_every_field_from_the_snapshot() {
        // Given a popup opened on specific values and edited since.
        let mut popup = AttendantPropertiesState {
            original: Some(OriginalValues {
                trigger: AttendantTrigger::ParentCompleted,
                activation: AttendantActivation::Preserve,
                template: "original".to_owned(),
            }),
            pending_trigger: AttendantTrigger::Manual,
            pending_activation: AttendantActivation::Seed,
            ..AttendantPropertiesState::default()
        };
        popup.seed_template.input = "edited".to_owned();

        // When restoring the original values.
        popup.restore_original();

        // Then every field is back to its open-time value.
        assert_eq!(popup.pending_trigger, AttendantTrigger::ParentCompleted);
        assert_eq!(popup.pending_activation, AttendantActivation::Preserve);
        assert_eq!(popup.seed_template.input, "original");
    }

    #[rstest::rstest]
    fn restore_original_is_a_noop_without_a_snapshot() {
        // Given a popup that was never opened.
        let mut popup = AttendantPropertiesState::default();

        // When restoring the original values.
        popup.restore_original();

        // Then the defaults stand.
        assert_eq!(popup.pending_trigger, AttendantTrigger::Manual);
        assert_eq!(popup.seed_template.input, "");
    }

    #[rstest::rstest]
    fn cancel_template_edit_restores_the_pre_editor_text() {
        // Given a popup whose editor captured a pre-edit text and whose
        // template has since changed.
        let mut popup = AttendantPropertiesState::default();
        popup.seed_template.input = "before".to_owned();
        popup.begin_template_edit();
        popup.seed_template.input = "during".to_owned();

        // When cancelling the editor.
        popup.cancel_template_edit();

        // Then the template is back to the pre-editor text.
        assert_eq!(popup.seed_template.input, "before");
        // And the editor is closed.
        assert!(popup.editor_original.is_none());
    }

    #[rstest::rstest]
    fn keep_template_edit_accepts_the_text_and_closes_the_editor() {
        // Given a popup whose editor changed the template text.
        let mut popup = AttendantPropertiesState::default();
        popup.begin_template_edit();
        popup.seed_template.input = "new text".to_owned();

        // When keeping the edit.
        popup.keep_template_edit();

        // Then the edited text stands.
        assert_eq!(popup.seed_template.input, "new text");
        // And the editor is closed.
        assert!(popup.editor_original.is_none());
    }
}
