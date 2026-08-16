//! Envelope — the frame every wire message travels in.

use serde::{Deserialize, Serialize};

/// The wire protocol major version this crate speaks.
///
/// A mismatched `v` in a received envelope makes the whole line untrusted;
/// receivers drop it and log. Within a major version, unknown message tags
/// are tolerated (see [`crate::wire`]).
pub const PROTOCOL_VERSION: u32 = 1;

/// One NDJSON line: envelope + message payload.
///
/// Both directions (host→plugin and plugin→host) use this shape. `seq` is
/// assigned monotonically per direction by the sender; v1 receivers do not
/// act on gaps (replay-since-seq is future work the field exists for).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    /// Wire protocol major version. Must equal [`PROTOCOL_VERSION`].
    pub v: u32,
    /// Sender-assigned monotonic sequence number (per direction).
    pub seq: u64,
    /// Unix epoch milliseconds when the sender produced the message.
    pub ts: u64,
    /// The message payload.
    #[serde(flatten)]
    pub msg: PluginToHostOrHostToPlugin,
}

/// Direction-erased envelope payload.
///
/// Serialization is by the `msg` field inside [`Envelope`]; this type only
/// exists so one envelope type serves both directions. Each side deserializes
/// with the enum it knows how to produce and relies on `#[serde(other)]`
/// tolerance for anything else.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PluginToHostOrHostToPlugin {
    /// A plugin→host message (contributions, handshake).
    Plugin(PluginToHost),
    /// A host→plugin message (handshake reply).
    Host(HostToPlugin),
}

impl Envelope {
    /// Wraps a plugin→host message with the given sequence number and a
    /// caller-supplied timestamp (tests inject fixed clocks).
    #[must_use]
    pub fn for_plugin(msg: PluginToHost, seq: u64, ts: u64) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            seq,
            ts,
            msg: PluginToHostOrHostToPlugin::Plugin(msg),
        }
    }

    /// Wraps a host→plugin message with the given sequence number and a
    /// caller-supplied timestamp.
    #[must_use]
    pub fn for_host(msg: HostToPlugin, seq: u64, ts: u64) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            seq,
            ts,
            msg: PluginToHostOrHostToPlugin::Host(msg),
        }
    }
}

use crate::wire::{HostToPlugin, PluginToHost};
