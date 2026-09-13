//! Discord-slice integration tests: the slice's rows inside the composed
//! system.
//!
//! These exercise the discord slice's route rows **as composed** — attached
//! alongside every other slice's rows, exactly as launch builds the keymap.
//! The `gdc` sequence, its derived which-key group label, and the guarantee
//! that slice rows never pierce typing are all coexistence properties: they
//! hold *because* the full row set is present, so the tests query the
//! full-composition keymap (`composed_keymap`).

#![allow(clippy::expect_used, clippy::panic, reason = "test code")]

use crate::common::{composed_keymap, plain};
use jinn_domain::Intent;
use jinn_tui::Scope;
use ratatui_which_key::NodeResult;

/// The composed `gdc` sequence resolves to discord's to-thread action:
/// the slice's row survives the merge into the composed keymap.
#[rstest::rstest]
#[test]
fn gdc_resolves_to_discord_to_thread_in_the_composed_keymap() {
    // Given the composed keymap (built-in bindings + every slice's rows).
    let keymap = composed_keymap();

    // When navigating the gdc sequence in the Normal scope.
    let result = keymap
        .navigate(&[plain('g'), plain('d'), plain('c')], &Scope::Normal)
        .expect("gdc path exists in the composed keymap");

    // Then it resolves to a dynamic intent for discord's to-thread action.
    let NodeResult::Leaf { action } = result else {
        panic!("gdc must be a leaf, got {result:?}");
    };
    let Intent::Dynamic(dynamic) = action else {
        panic!("gdc must resolve to a dynamic intent, got {action:?}");
    };
    assert_eq!(dynamic.slice.key(), "discord:actions");
    assert_eq!(dynamic.action, "to-thread");
}

/// The `gd` prefix under `g` derives a group labeled "discord" (the
/// feature label), while the root `g` keeps its hardcoded "general" label.
#[rstest::rstest]
#[test]
fn gd_prefix_derives_the_discord_group_label() {
    // Given the composed keymap.
    let keymap = composed_keymap();

    // When listing the children under the `g` prefix in Normal scope.
    let g_children = keymap
        .children_at_path(&[plain('g')], &Scope::Normal)
        .expect("g group bindings");

    // Then the `d` child is described as the discord group.
    assert!(
        g_children
            .iter()
            .any(|b| b.key == plain('d') && b.description == "discord"),
        "gd group should be derived with the discord label, got {g_children:?}"
    );
    // And the root `g` keeps its hardcoded "general" description.
    let root = keymap
        .children_at_path(&[], &Scope::Normal)
        .expect("root bindings");
    assert!(
        root.iter()
            .any(|b| b.key == plain('g') && b.description == "general"),
        "root g should keep the general label, got {root:?}"
    );
}

/// The discord row does not pierce typing: no `g` branch exists in the
/// Input scope even with every slice's rows attached.
#[rstest::rstest]
#[test]
fn gdc_is_absent_from_the_input_scope() {
    // Given the composed keymap.
    let keymap = composed_keymap();

    // When navigating the g prefix in the Input scope.
    let result = keymap.navigate(&[plain('g')], &Scope::Input);

    // Then nothing resolves — typing is untouched by slice rows.
    assert!(
        result.is_none(),
        "gdc must not bind in Input; got {result:?}"
    );
}
