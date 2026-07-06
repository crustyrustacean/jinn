//! Discord bot frontend — drive a running jinn instance from Discord.
//!
//! This feature provides the domain-layer pieces for the Discord bot:
//! the bridge actor (bus → channel forwarder), the thread-map DAO, config,
//! the message splitter, final-reply extraction, and the shared wire
//! protocol. The poise gateway itself (Discord websocket + slash commands)
//! lives in a separate `jinn-discord` crate so the domain never depends on
//! serenity.

pub mod bridge_actor;
pub mod config;
pub mod message_split;
pub mod protocol;
pub mod reply;
pub mod repo_basename;
pub mod route;
pub mod thread_map;
pub mod feedback_actor;
pub mod to_thread_intent;

pub use bridge_actor::{DiscordBridgeActor, DiscordBridgeActorDeps};
pub use config::DiscordConfig;
pub use message_split::split_message;
pub use protocol::{
    BridgeEvent,
    CreateThreadForSession,
    CreateThreadReason,
    DiscordThreadCreateFailed,
    DiscordThreadCreated,
    GatewayRequest,
    ThreadId,
};
pub use reply::{FinalReply, read_final_reply};
pub use repo_basename::repo_basename;
pub use route::{RouteDecision, route_decision};
pub use thread_map::{DiscordThreadMap, DiscordThreadMapError, ThreadMapping};

#[cfg(test)]
mod tests;
