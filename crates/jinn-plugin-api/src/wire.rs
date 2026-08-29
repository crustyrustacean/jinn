//! Wire messages — thin tagged unions over individual structs.
//!
//! The structs ([`Hello`], [`SetThemeEntries`], ...) are the source of truth:
//! each is versioned, tested, and evolves independently. The enums exist only
//! as transport unions so a receiver can discriminate one line without
//! knowing the type ahead of time, and `#[serde(other)]` on the
//! direction-erased wrapper gives forward compatibility: a tag this build
//! doesn't know deserializes to `Unknown` instead of erroring.
//!
//! # Forward-compatibility caveat
//!
//! `#[serde(other)]` requires a unit variant, so an unknown tag with a data
//! payload degrades to `Unknown` — the payload is dropped. That is the
//! accepted trade: within a major version, messages only ever get added, and
//! receivers ignore what they don't understand rather than failing.

use serde::{Deserialize, Serialize};

use crate::persona_def::PersonaDef;
use crate::theme_def::ThemeDef;

/// Subscription kinds a plugin may declare in [`Hello::subscriptions`].
///
/// Each tag names one host→guest event kind; the host forwards matching
/// events to subscribed guests. Unknown tags are warned about and ignored
/// (forward compatibility — a newer guest's new tags must not break an
/// older host).
pub const SUBSCRIPTION_KINDS: &[&str] = &["tool_call", "tool_result", "turn_end"];

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

/// Contribution: the full set of persona definitions the plugin knows about.
///
/// Push, never pull — the plugin sends this on start and again whenever its
/// view changes. The host translates and publishes them as loaded personas;
/// opening the persona picker never queries the plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetPersonaEntries {
    /// Complete set of personas (a full replacement, not a delta).
    pub personas: Vec<PersonaDef>,
}

/// Event: a complete tool call the model produced (arguments assembled).
///
/// Forwarded to plugins subscribed to `"tool_call"`. Carries the raw
/// argument JSON so guests can run their own shape detection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallEvent {
    /// The session the tool call belongs to (opaque id string).
    pub session_id: String,
    /// Unique identifier for this tool call (assigned by the LLM provider).
    pub tool_call_id: String,
    /// The tool's (possibly namespaced) name.
    pub name: String,
    /// The arguments as a JSON string, exactly as the model produced them.
    pub arguments: String,
}

/// Event: a tool execution finished (builtin or MCP alike).
///
/// Forwarded to plugins subscribed to `"tool_result"`. The content is the
/// untruncated output when truncation occurred (`full_content`), so JSON
/// payloads always parse on the guest side.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultEvent {
    /// The session the execution belongs to (opaque id string).
    pub session_id: String,
    /// The tool call this result answers.
    pub tool_call_id: String,
    /// The tool's (possibly namespaced) name.
    pub name: String,
    /// The execution output (untruncated when the host had it).
    pub content: String,
    /// Whether execution succeeded. Failed executions are not citable.
    pub success: bool,
}

/// Event: a session turn ended (streaming → idle).
///
/// Forwarded to plugins subscribed to `"turn_end"`. `final_answer` is
/// host-computed (the session's last history entry is an assistant
/// message); guests cannot read session state themselves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnEndEvent {
    /// The session whose turn ended (opaque id string).
    pub session_id: String,
    /// Whether the turn reached a genuine final assistant answer. `false`
    /// means error/cancel mid-turn — buffered turn-scoped state should be
    /// retained for a later successful turn.
    pub final_answer: bool,
}

/// One citation contributed by a plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginCitation {
    /// The source URL (must be http/https — anything else is dropped).
    pub url: String,
    /// A human-readable title for the source.
    pub title: String,
    /// Optional snippet text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// Contribution: the citations a plugin accumulated over a turn.
///
/// Sent in response to [`TurnEndEvent`] when the turn reached a final
/// answer. The host validates each entry and republishes the survivors as
/// the session's grouped `Sources` footer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushCitations {
    /// The session the citations belong to (opaque id string).
    pub session_id: String,
    /// The accumulated citations for the turn.
    pub citations: Vec<PluginCitation>,
}

/// Request: cancel the active provider stream for a session.
///
/// A message-style mirror of the host's internal `CancelStream` bus
/// command — same effect, public wire shape. Canceling an idle session is
/// a host-side no-op, so duplicate sends are harmless.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CancelStream {
    /// The session whose active stream should be canceled (opaque id).
    pub session_id: String,
}

/// Request: insert one system-kind chat entry at the end of a session's
/// history.
///
/// A message-style mirror of the host's internal
/// `SubmitHistoryMutations(InsertEntry)` tail-append flow — same effect,
/// public wire shape. The host renders it like any other system entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertSystemEntry {
    /// The session to append to (opaque id string).
    pub session_id: String,
    /// The system entry text.
    pub text: String,
}

/// Plugin→host message union (transport only).
///
/// Unknown tags are handled one level up, in
/// [`PluginToHostOrHostToPlugin`] — an `#[serde(other)]` arm here would
/// shadow the host direction entirely (untagged tries `Plugin` first).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginToHost {
    /// Handshake opener.
    Hello(Hello),
    /// Theme contribution (full set).
    SetThemeEntries(SetThemeEntries),
    /// Persona contribution (full set).
    SetPersonaEntries(SetPersonaEntries),
    /// Citation contribution (turn-scoped).
    PushCitations(PushCitations),
    /// Cancel the active provider stream for a session.
    CancelStream(CancelStream),
    /// Insert a system entry at the end of a session's history.
    InsertSystemEntry(InsertSystemEntry),
}

/// Host→plugin message union (transport only).
///
/// Unknown tags are handled one level up, in
/// [`PluginToHostOrHostToPlugin`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostToPlugin {
    /// Handshake reply.
    Welcome(Welcome),
    /// A complete tool call arrived (subscribed event).
    ToolCallEvent(ToolCallEvent),
    /// A tool execution finished (subscribed event).
    ToolResultEvent(ToolResultEvent),
    /// A session turn ended (subscribed event).
    TurnEndEvent(TurnEndEvent),
}
