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
/// One envelope type serves both directions: the payload's `type` tag
/// decides which. Serialization delegates to the inner enum (each carries
/// its tag). Deserialization dispatches on the tag: a tag neither enum
/// defines degrades to [`PluginToHostOrHostToPlugin::Unknown`] — the
/// payload is dropped, the line is not an error (forward compatibility;
/// see [`crate::wire`]).
#[derive(Debug, Clone, PartialEq)]
pub enum PluginToHostOrHostToPlugin {
    /// A plugin→host message (contributions, handshake).
    Plugin(PluginToHost),
    /// A host→plugin message (handshake reply).
    Host(HostToPlugin),
    /// Unknown tag — payload dropped.
    Unknown,
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

// ── Serde for the direction-erased payload ───────────────────────────────────

impl Serialize for PluginToHostOrHostToPlugin {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Plugin(msg) => msg.serialize(serializer),
            Self::Host(msg) => msg.serialize(serializer),
            // An empty object carries no tag; receivers degrade it to Unknown.
            Self::Unknown => {
                use serde::ser::SerializeMap as _;
                serializer.serialize_map(Some(0))?.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for PluginToHostOrHostToPlugin {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let tag = value.get("type").and_then(serde_json::Value::as_str);
        let Some(tag) = tag else {
            return Ok(Self::Unknown);
        };
        if tag == "welcome" {
            return HostToPlugin::deserialize(value)
                .map(Self::Host)
                .map_err(serde::de::Error::custom);
        }
        if matches!(tag, "hello" | "set_theme_entries" | "set_persona_entries") {
            return PluginToHost::deserialize(value)
                .map(Self::Plugin)
                .map_err(serde::de::Error::custom);
        }
        Ok(Self::Unknown)
    }
}
