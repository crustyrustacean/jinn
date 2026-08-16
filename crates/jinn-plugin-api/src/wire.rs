//! Wire messages — thin tagged unions over individual structs.
//!
//! The structs ([`Hello`], [`SetThemeEntries`], ...) are the source of truth:
//! each is versioned, tested, and evolves independently. The enums exist only
//! as transport unions so a receiver can discriminate one line without
//! knowing the type ahead of time, and so `#[serde(other)]` gives forward
//! compatibility: a tag this build doesn't know deserializes to `Unknown`
//! instead of erroring.
//!
//! # Forward-compatibility caveat
//!
//! `#[serde(other)]` requires a unit variant, so an unknown tag with a data
//! payload degrades to `Unknown` — the payload is dropped. That is the
//! accepted trade: within a major version, messages only ever get added, and
//! receivers ignore what they don't understand rather than failing.

use serde::{Deserialize, Serialize};

use crate::theme_def::ThemeDef;

/// Handshake: the first message a plugin sends after boot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    /// Wire protocol major version the plugin speaks.
    pub protocol_version: u32,
    /// Human-readable plugin name (from the manifest).
    pub name: String,
    /// Event types the plugin subscribes to (v1: none exist; the field is
    /// part of the schema so future host→plugin events need no shape change).
    #[serde(default)]
    pub subscriptions: Vec<String>,
}

/// Handshake reply: the host's answer to [`Hello`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Welcome {
    /// Wire protocol major version the host speaks.
    pub protocol_version: u32,
    /// The id the host assigned to this plugin instance (the manifest name).
    pub plugin_id: String,
    /// Filesystem paths the plugin is allowed to read (absolute, resolved).
    #[serde(default)]
    pub read_dirs: Vec<String>,
    /// Directories the plugin may write to (absolute, resolved).
    #[serde(default)]
    pub write_dirs: Vec<String>,
    /// Whether the plugin may make network requests.
    pub http_allowed: bool,
    /// Plugin-specific configuration table (free-form, from the manifest).
    #[serde(default)]
    pub config: serde_json::Value,
}

/// Contribution: the full set of theme definitions the plugin knows about.
///
/// Push, never pull — the plugin sends this on start and again whenever its
/// view changes. Opening the theme picker never queries the plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetThemeEntries {
    /// Complete set of themes (a full replacement, not a delta).
    pub themes: Vec<ThemeDef>,
}

/// Plugin→host message union (transport only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginToHost {
    /// Handshake opener.
    Hello(Hello),
    /// Theme contribution (full set).
    SetThemeEntries(SetThemeEntries),
    /// Unknown tag — payload dropped. See module docs.
    #[serde(other)]
    Unknown,
}

/// Host→plugin message union (transport only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostToPlugin {
    /// Handshake reply.
    Welcome(Welcome),
    /// Unknown tag — payload dropped. See module docs.
    #[serde(other)]
    Unknown,
}
