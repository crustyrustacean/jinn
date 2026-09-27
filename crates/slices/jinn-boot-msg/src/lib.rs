//! Boot crossing contracts.
//!
//! The EXPORT surface of the boot slice: the readiness event published
//! by composition after every actor has spawned, and the env-config ask
//! path (`GetEnvironmentConfig` → `EnvironmentConfigReply`) composition
//! walks to kick off the startup chain.
//!
//! Kernel consumers of [`EnvironmentLoaded`]: the session actor
//! (session bootstrapping on config receipt). The ask path is consumed
//! by composition itself (`actor_wiring`'s startup tail).

use jinn_provider_config::ProvidersConfig;
use serde::{Deserialize, Serialize};

/// All actors have been spawned.
///
/// Emitted after the wiring code finishes spawning every actor. The
/// system-ready actor waits for this event, then releases the oneshot
/// that gates the TUI's first draw — it does no counting of its own.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "All actors have been spawned; the system is ready.")]
pub struct AllActorsSpawned;

impl jinn_slices::BusMessage for AllActorsSpawned {}

/// The environment has been loaded and API keys are available.
///
/// Emitted after the env init actor has populated `ApiKeysService`.
/// Published at runtime for environment reloads (not during startup).
/// Downstream actors should use `ask(GetEnvironmentConfig)` for initial config.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "The environment has been loaded and API keys are available.")]
pub struct EnvironmentLoaded {
    /// The parsed provider configuration from `providers.toml`.
    pub config: ProvidersConfig,
}

impl jinn_slices::BusMessage for EnvironmentLoaded {}

/// Ask message to retrieve the loaded environment config.
///
/// Downstream actors use this during their `on_start` to pull config
/// directly from the EnvInitActor via the actor registry.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Ask the env-init actor for the parsed provider configuration.")]
pub struct GetEnvironmentConfig;

impl jinn_slices::BusMessage for GetEnvironmentConfig {}

/// The reply payload of the `GetEnvironmentConfig` ask (JSON-friendly twin
/// of `Option<ProvidersConfig>`).
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "Reply payload for the GetEnvironmentConfig ask.")]
pub struct EnvironmentConfigReply {
    /// The loaded config, or `None` when the file is missing/unreadable.
    pub config: Option<ProvidersConfig>,
}

impl jinn_slices::BusMessage for EnvironmentConfigReply {}
