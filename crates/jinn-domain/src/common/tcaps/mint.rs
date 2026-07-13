//! Mint: the single trust point where caps are constructed.
//!
//! Every cap originates here. `rg "mint_"` lists every mint site. Only actor
//! wiring (`actor_wiring.rs`) calls these functions.
//!
//! Caps are ZSTs with private constructors scoped `pub(in crate::common::tcaps)`,
//! so this module is the *only* code that can construct them.

use crate::common::tcaps::context::ContextCap;
use crate::common::tcaps::discovered_plugins::DiscoveredPluginsCap;
use crate::common::tcaps::frontend::FrontendCap;
use crate::common::tcaps::provider::ProviderCap;
use crate::common::tcaps::session::SessionCap;

/// Mint a [`ProviderCap`]. Called from actor wiring.
pub fn mint_provider_cap() -> ProviderCap {
    ProviderCap::new()
}

/// Mint a [`DiscoveredPluginsCap`]. Called from actor wiring (startup-only write).
pub fn mint_discovered_plugins_cap() -> DiscoveredPluginsCap {
    DiscoveredPluginsCap::new()
}

/// Mint a [`FrontendCap`]. Called from actor wiring.
pub fn mint_frontend_cap() -> FrontendCap {
    FrontendCap::new()
}

/// Mint a [`ContextCap`]. Called from actor wiring.
pub fn mint_context_cap() -> ContextCap {
    ContextCap::new()
}

/// Mint a [`SessionCap`]. Called from actor wiring.
pub fn mint_session_cap() -> SessionCap {
    SessionCap::new()
}
