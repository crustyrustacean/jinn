//! Mint: the single trust point where caps are constructed.
//!
//! Every cap originates here. `rg "mint_"` lists every mint site. Only actor
//! wiring (`actor_wiring.rs`) calls these functions.
//!
//! Caps are ZSTs with private constructors scoped `pub(in crate::common::tcaps)`,
//! so this module is the *only* code that can construct them.

use crate::common::tcaps::discovered_plugins::DiscoveredPluginsCap;
use crate::common::tcaps::provider::ProviderCap;

/// Mint a [`ProviderCap`]. Called from actor wiring.
pub fn mint_provider_cap() -> ProviderCap {
    ProviderCap::new()
}

/// Mint a [`DiscoveredPluginsCap`]. Called from actor wiring (startup-only write).
pub fn mint_discovered_plugins_cap() -> DiscoveredPluginsCap {
    DiscoveredPluginsCap::new()
}
