//! Discovered-plugins capsule: cap + view for the startup-only write to
//! `AppState.discovered_plugins`.
//!
//! Write access to `Vec<DiscoveredPlugin>` is gated by an unforgeable ZST token
//! ([`DiscoveredPluginsCap`]). The projection method
//! [`State::with_discovered_plugins`] hands the cap-holder a narrow borrowed
//! view ([`DiscoveredPluginsView`]).

use crate::common::app_state::DiscoveredPlugin;
use crate::common::state::State;

// ── The cap ──────────────────────────────────────────────────────────────────

/// Proof of authority to write the discovered-plugins list. Minted only via
/// [`crate::common::tcaps::mint`].
#[derive(Clone, Copy, Debug)]
pub struct DiscoveredPluginsCap(());

impl DiscoveredPluginsCap {
    /// Private constructor scoped to the `tcaps/` subtree.
    pub(in crate::common::tcaps) fn new() -> Self {
        Self(())
    }
}

// ── Per-struct narrow newtype ───────────────────────────────────────────────

/// Narrow write-handle to `Vec<DiscoveredPlugin>`. The tuple field is PRIVATE
/// — reaching `.0` from a consumer is a compile error. Only the [`DiscoveredPluginsWrite`]
/// trait method is reachable.
pub struct DiscoveredPluginsOps<'a>(&'a mut Vec<DiscoveredPlugin>);

impl DiscoveredPluginsOps<'_> {
    /// Replace the entire list.
    pub fn set(&mut self, plugins: Vec<DiscoveredPlugin>) {
        self.0.clear();
        self.0.extend(plugins);
    }
    /// Read-only access to the current list.
    pub fn discovered_plugins(&self) -> &[DiscoveredPlugin] {
        self.0
    }
}

// ── Composite facade ─────────────────────────────────────────────────────────

/// What a discovered-plugins-writer sees: mutable access to the plugins list.
pub struct DiscoveredPluginsView<'a> {
    /// Mutable discovered-plugins list, scoped via [`DiscoveredPluginsOps`].
    pub discovered_plugins: DiscoveredPluginsOps<'a>,
}

// ── Projection method ────────────────────────────────────────────────────────

impl State {
    /// Write access to the discovered-plugins list, scoped via
    /// [`DiscoveredPluginsView`].
    pub fn with_discovered_plugins<R, F>(&self, _cap: &DiscoveredPluginsCap, f: F) -> R
    where
        F: FnOnce(&mut DiscoveredPluginsView<'_>) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut DiscoveredPluginsView {
            discovered_plugins: DiscoveredPluginsOps(&mut app.discovered_plugins),
        })
    }
}
