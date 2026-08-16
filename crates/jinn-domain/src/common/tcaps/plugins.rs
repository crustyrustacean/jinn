//! Plugins capsule: cap + view, colocated.
//!
//! Write access to the plugin contribution cache
//! ([`PluginContributions`](crate::feat::plugin::PluginContributions)) is
//! gated by an unforgeable ZST token ([`PluginsCap`]). The projection
//! method [`State::with_plugins`] hands the cap-holder a mutable view.
//!
//! The single legitimate writer is the plugin coordinator actor: it is the
//! trust boundary where inbound plugin messages are validated, and it owns
//! both the contribution cache and the plugin phase map.

use crate::common::state::State;
use crate::feat::plugin::PluginContributions;

// ── The cap ──────────────────────────────────────────────────────────────────

/// Proof of authority to write the plugin contribution cache. Minted only
/// via [`crate::common::tcaps::mint`].
#[derive(Clone, Copy, Debug)]
pub struct PluginsCap(());

impl PluginsCap {
    /// Private constructor scoped to the `tcaps/` subtree.
    pub(in crate::common::tcaps) fn new() -> Self {
        Self(())
    }
}

// ── The view ─────────────────────────────────────────────────────────────────

impl State {
    /// Write access to the plugin contribution cache.
    pub fn with_plugins<R, F>(&self, _cap: &PluginsCap, f: F) -> R
    where
        F: FnOnce(&mut PluginContributions) -> R,
    {
        let mut guard = self.write_lock();
        f(&mut guard.plugins)
    }
}
