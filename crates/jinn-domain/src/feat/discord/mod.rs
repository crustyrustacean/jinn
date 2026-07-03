//! This feature provides the domain-layer pieces for the Discord bot:
//! the bridge actor (bus → channel forwarder), the thread-map DAO, config,
//! and the shared wire protocol. The poise gateway itself (Discord
//! websocket + slash commands) lives in a separate `jinn-discord` crate so
//! the domain never depends on serenity.

pub mod bridge_actor;
pub mod config;
pub mod protocol;
pub mod thread_map;

pub use bridge_actor::{DiscordBridgeActor, DiscordBridgeActorDeps};
pub use config::DiscordConfig;
pub use protocol::{BridgeEvent, ThreadId};
pub use thread_map::{DiscordThreadMap, DiscordThreadMapError, ThreadMapping};

#[cfg(test)]
mod tests;
