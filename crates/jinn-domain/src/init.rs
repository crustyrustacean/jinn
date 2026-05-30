//! Application initialization actors.
//!
//! This module owns all startup file I/O and environment resolution.
//! Actors here run before domain actors need their data, establishing
//! a dependency chain through events:
//!
//! 1. `env_init_actor` - reads env vars, populates API keys, emits `EnvironmentLoaded`
//! 2. `provider_init_actor` - on `EnvironmentLoaded`, loads providers, merges cache, resolves `last_model`

pub mod env_init_actor;
pub mod provider_init_actor;
pub mod system_ready_actor;

pub use env_init_actor::EnvironmentLoaded;
