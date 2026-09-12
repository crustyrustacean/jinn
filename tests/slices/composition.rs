//! Composition-seam integration tests: the shared seam itself, not any
//! single slice.
//!
//! These assert what composition as a whole must provide — that the
//! composed route table carries every in-tree slice's rows. A missing
//! slice here means its `activate()` never attached rows, so nothing
//! downstream (keymap, which-key, rendering) can see it.

#![allow(clippy::expect_used, clippy::panic, reason = "test code")]

use crate::common::composition_routes;
use jinn_dashboard::dashboard_scope;
use jinn_quake_bar::quake_scope;

/// The row seam carries every slice's rows: dashboard, quake-bar, and
/// discord all attached (the precondition the keymap tests rely on).
#[rstest::rstest]
#[test]
fn composition_sees_rows_from_every_slice() {
    // Given the composed route table.
    let routes = composition_routes();

    // When listing the dynamic scopes it knows about.
    let scopes = jinn_tui::keymap_gen::dynamic_scopes(&routes);

    // Then every slice's scope is present.
    assert!(
        scopes.iter().any(|s| *s == dashboard_scope()),
        "dashboard scope missing from composed routes"
    );
    assert!(
        scopes.iter().any(|s| *s == quake_scope()),
        "quake-bar scope missing from composed routes"
    );
    assert!(
        scopes.iter().any(|s| *s == jinn_discord::discord_scope()),
        "discord scope missing from composed routes"
    );
}
