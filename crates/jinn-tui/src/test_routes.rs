//! Composition-side keymap test fixture.
//!
//! A [`KeyRoutes`] pre-seeded with every slice's rows, mirroring what
//! composition produces at launch (all `activate()` calls made).
//!
//! Test-only seam: keymap tests query slice keys without standing up
//! the actor system. It lives in jinn-tui (the composition-adjacent
//! crate) because it attaches slice rows — the kernel's `jinn-domain`
//! must not import slice crates.

use jinn_domain::common::slices::key_routes::KeyRoutes;

/// A `KeyRoutes` pre-seeded with every slice's rows, mirroring what
/// composition produces at launch (all `activate()` calls made).
///
/// # Panics
///
/// Panics if the detached quake cell cannot be minted (a fresh
/// `Slices` never has it registered, so this is unreachable).
#[must_use]
pub fn composition_routes() -> KeyRoutes {
    let routes = KeyRoutes::new();
    jinn_dashboard::attach_dashboard_rows(&routes);
    // The quake rows' submit/scroll actions capture a cell handle; the
    // seam mints a detached one (never registered into a live `Slices`)
    // since only row *shape* matters for keymap tests.
    let slices = jinn_domain::common::slices::Slices::new();
    #[expect(
        clippy::expect_used,
        reason = "test seam: a fresh Slices never has the quake cell registered"
    )]
    let cell = slices
        .register(
            jinn_quake_bar::quake_bar_slot(),
            jinn_quake_bar::QuakeBarState::default(),
        )
        .expect("fresh Slices never has the quake cell registered");
    jinn_quake_bar::attach_quake_bar_rows(&routes, &cell);
    jinn_quake_bar::register_quake_input_hook(&routes, &cell);
    // The discord rows attach through the slice's own activation
    // (`jinn_discord_slice::activate`) — tests that need them call
    // `jinn_discord_slice::attach_discord_rows` directly.
    routes
}
