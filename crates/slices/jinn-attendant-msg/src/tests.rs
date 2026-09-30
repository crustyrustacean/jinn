#[cfg(test)]
mod attendant_msg_tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, reason = "test code")]

    use jiff::Timestamp;

    use crate::{AttendantBehavior, AttendantReport, AttendantTrigger, PRIOR_REPORT_PLACEHOLDER};

    #[rstest::rstest]
    fn the_preserve_behavior_is_named_preserve_on_disk() {
        // Given the behavior that keeps the prior conversation intact.
        let behavior = AttendantBehavior::Preserve;

        // When it is serialized as stored in session metadata.
        let json = serde_json::to_string(&behavior).expect("serializes");

        // Then the stored name is `preserve`. The name has to be this, not
        // `continue`: the `c` key already means "continue this session", and
        // one word carrying two meanings is how a behavior gets misread.
        assert_eq!(json, r#""preserve""#);
        // And it round-trips back to the same behavior.
        assert_eq!(
            serde_json::from_str::<AttendantBehavior>(&json).expect("deserializes"),
            behavior
        );
    }

    #[rstest::rstest]
    fn behavior_defaults_to_reset() {
        // Given no explicit behavior.

        // When creating the default behavior.
        let behavior = AttendantBehavior::default();

        // Then a run rebuilds its context from the pins alone.
        assert_eq!(behavior, AttendantBehavior::Reset);
    }

    #[rstest::rstest]
    #[case(AttendantBehavior::Reset, true)]
    #[case(AttendantBehavior::Preserve, false)]
    fn only_reset_rebuilds_context(#[case] behavior: AttendantBehavior, #[case] expected: bool) {
        // Given a behavior.

        // When asking whether a run in it rebuilds context.
        let resets = behavior.resets_context();

        // Then only reset excludes the history: keeping the context as it
        // stands is the whole of the other behavior.
        assert_eq!(resets, expected);
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
        AttendantBehavior, AttendantPropertiesState, AttendantTrigger, BEHAVIOR_CHOICES,
        OriginalValues, PickDirection, PropertyField, TRIGGER_CHOICES, attendant_properties_scope,
        attendant_seed_template_scope, pick_behavior, pick_trigger,
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
    fn behavior_choices_are_ordered_reset_preserve() {
        // Given the behavior choice row.

        // When reading its values in display order.
        let values: Vec<_> = BEHAVIOR_CHOICES.iter().map(|(v, _)| *v).collect();

        // Then the row reads reset, preserve.
        assert_eq!(
            values,
            vec![AttendantBehavior::Reset, AttendantBehavior::Preserve]
        );
    }

    #[rstest::rstest]
    fn the_form_has_its_four_rows_in_display_order() {
        // Given a popup whose cursor is on the first row.

        // When moving down one row per press.
        let mut popup = AttendantPropertiesState {
            pending_prep_mode: false,
            ..AttendantPropertiesState::default()
        };
        let mut visited = vec![popup.focus];
        for _ in 0..3 {
            popup.focus_next();
            visited.push(popup.focus);
        }

        // Then the rows are the four the panel shows, in the order it shows
        // them: what fires, what it sees, whether it is still being
        // composed, and the text a run injects.
        assert_eq!(
            visited,
            vec![
                PropertyField::Trigger,
                PropertyField::Behavior,
                PropertyField::PrepMode,
                PropertyField::SeedTemplate,
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
        // Given the rightmost behavior choice.

        // When picking right.
        let picked = pick_behavior(AttendantBehavior::Preserve, PickDirection::Right);

        // Then the choice does not move.
        assert_eq!(picked, AttendantBehavior::Preserve);
    }

    #[rstest::rstest]
    fn picking_moves_one_choice_per_key() {
        // Given a mid-row trigger choice.

        // When picking in each direction.
        let left = pick_trigger(AttendantTrigger::ParentCompleted, PickDirection::Left);
        let right = pick_trigger(AttendantTrigger::ParentCompleted, PickDirection::Right);

        // Then each pick lands on the adjacent choice.
        assert_eq!(left, AttendantTrigger::ParentCompleted);
        assert_eq!(right, AttendantTrigger::Manual);
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
            pending_prep_mode: false,
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
            pending_prep_mode: false,
            ..AttendantPropertiesState::default()
        };
        popup.focus_previous();

        // Then the cursor stays put.
        assert_eq!(popup.focus, PropertyField::Trigger);
    }

    #[rstest::rstest]
    fn the_cursor_cannot_reach_the_rows_above_a_prepping_attendant() {
        // Given a popup whose attendant is in prep mode, cursor on the
        // template row.
        let mut popup = AttendantPropertiesState {
            focus: PropertyField::SeedTemplate,
            pending_prep_mode: true,
            ..AttendantPropertiesState::default()
        };

        // When moving up twice — enough to cross both dimmed rows.
        popup.focus_previous();
        popup.focus_previous();
        popup.focus_previous();

        // Then the cursor never lands above the prep row. The rows are on
        // screen but do not apply, so a cursor on one of them would be on a
        // control that cannot be operated.
        assert_eq!(popup.focus, PropertyField::PrepMode);
    }

    #[rstest::rstest]
    fn the_cursor_returns_to_the_dimmed_rows_once_prep_mode_is_off() {
        // Given a popup whose attendant has left prep mode, cursor on the
        // template row.
        let mut popup = AttendantPropertiesState {
            focus: PropertyField::SeedTemplate,
            pending_prep_mode: false,
            ..AttendantPropertiesState::default()
        };

        // When moving up three times — once per remaining row.
        for _ in 0..3 {
            popup.focus_previous();
        }

        // Then the cursor reaches the top row: leaving prep mode makes the
        // two rows above live again, and one press is still one row.
        assert_eq!(popup.focus, PropertyField::Trigger);
    }

    #[rstest::rstest]
    fn a_composing_attendant_opens_with_the_cursor_on_the_prep_row() {
        // Given prep mode on and off.

        // When asking where the cursor rests as the popup opens.
        let prepping = PropertyField::opening_focus(true);
        let running = PropertyField::opening_focus(false);

        // Then a composing attendant opens on the prep row — the only row
        // above which two inapplicable rows sit — and a running one opens on
        // the first row as before.
        assert_eq!(prepping, PropertyField::PrepMode);
        assert_eq!(running, PropertyField::Trigger);
    }

    #[rstest::rstest]
    fn the_seed_template_still_applies_while_prepping() {
        // Given each field.

        // When asking whether it applies during composition.
        let applies = [
            PropertyField::Trigger,
            PropertyField::Behavior,
            PropertyField::PrepMode,
            PropertyField::SeedTemplate,
        ]
        .map(|field| field.applies_while_prepping());

        // Then pins are the point of composing, so the template is exactly
        // what a user writes during prep; only the two run settings do not
        // apply.
        assert_eq!(applies, [false, false, true, true]);
    }

    #[rstest::rstest]
    fn pick_moves_the_focused_choice_row_only() {
        // Given a popup focused on the trigger with pending edits on both
        // rows and prep mode off.
        let mut popup = AttendantPropertiesState {
            focus: PropertyField::Trigger,
            pending_behavior: AttendantBehavior::Preserve,
            pending_trigger: AttendantTrigger::ParentCompleted,
            pending_prep_mode: false,
            ..AttendantPropertiesState::default()
        };

        // When picking right.
        popup.pick(PickDirection::Right);

        // Then the trigger moved to its rightmost choice.
        assert_eq!(popup.pending_trigger, AttendantTrigger::Manual);
        // And the behavior did not move.
        assert_eq!(popup.pending_behavior, AttendantBehavior::Preserve);

        // Given the same popup refocused on the behavior.
        popup.focus = PropertyField::Behavior;

        // When picking left.
        popup.pick(PickDirection::Left);

        // Then the behavior moved one choice left.
        assert_eq!(popup.pending_behavior, AttendantBehavior::Reset);
        // And the trigger did not move.
        assert_eq!(popup.pending_trigger, AttendantTrigger::Manual);
    }

    #[rstest::rstest]
    #[case(PickDirection::Left, "h")]
    #[case(PickDirection::Right, "l")]
    fn either_pick_key_cycles_prep_mode(#[case] direction: PickDirection, #[case] _key: &str) {
        // Given a popup focused on the prep row, still composing.
        let mut popup = AttendantPropertiesState {
            focus: PropertyField::PrepMode,
            pending_prep_mode: true,
            ..AttendantPropertiesState::default()
        };

        // When picking.
        popup.pick(direction);

        // Then prep mode is off. A two-state field has no previous or next,
        // so both keys cycle it the same way.
        assert!(!popup.pending_prep_mode);
    }

    #[rstest::rstest]
    fn pick_does_not_change_the_run_settings_while_prepping() {
        // Given a popup whose trigger and behavior rows are focused in turn
        // while the attendant is composing.
        let mut popup = AttendantPropertiesState {
            pending_behavior: AttendantBehavior::Preserve,
            pending_trigger: AttendantTrigger::Manual,
            pending_prep_mode: true,
            ..AttendantPropertiesState::default()
        };
        popup.focus = PropertyField::Trigger;
        popup.pick(PickDirection::Right);
        popup.focus = PropertyField::Behavior;
        popup.pick(PickDirection::Left);

        // Then neither value moved: they are written down, they simply do
        // not govern anything until composition ends.
        assert_eq!(popup.pending_trigger, AttendantTrigger::Manual);
        assert_eq!(popup.pending_behavior, AttendantBehavior::Preserve);
    }

    #[rstest::rstest]
    fn pick_is_a_noop_on_the_seed_template_field() {
        // Given a popup focused on the seed template.
        let mut popup = AttendantPropertiesState {
            pending_prep_mode: false,
            ..AttendantPropertiesState::default()
        };

        // When picking in either direction.
        popup.pick(PickDirection::Left);
        popup.pick(PickDirection::Right);

        // Then nothing changed: the template is not a choice row.
        assert_eq!(popup.pending_trigger, AttendantTrigger::Manual);
        assert_eq!(popup.pending_behavior, AttendantBehavior::Reset);
    }

    #[rstest::rstest]
    fn restore_original_restores_every_field_from_the_snapshot() {
        // Given a popup opened on specific values and edited since.
        let mut popup = AttendantPropertiesState {
            original: Some(OriginalValues {
                trigger: AttendantTrigger::ParentCompleted,
                behavior: AttendantBehavior::Preserve,
                prep_mode: false,
                template: "original".to_owned(),
            }),
            pending_trigger: AttendantTrigger::Manual,
            pending_behavior: AttendantBehavior::Reset,
            pending_prep_mode: true,
            ..AttendantPropertiesState::default()
        };
        popup.seed_template.input = "edited".to_owned();

        // When restoring the original values.
        popup.restore_original();

        // Then every field is back to its open-time value.
        assert_eq!(popup.pending_trigger, AttendantTrigger::ParentCompleted);
        assert_eq!(popup.pending_behavior, AttendantBehavior::Preserve);
        assert!(!popup.pending_prep_mode);
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

    #[rstest::rstest]
    fn clearing_the_status_withdraws_the_armed_overwrite() {
        // Given a popup armed to overwrite a saved attendant.
        let mut popup = AttendantPropertiesState::default();
        popup.arm_save();

        // When any keystroke clears the status line.
        popup.clear_status();

        // Then the arm goes with it: a save the user was never told about
        // must not be one keystroke away.
        assert!(!popup.save_armed);
    }
}
