//! Conversation data model for the chat log.
//!
//! Each [`ChatEntry`] records a timestamped message from the user,
//! the system, or an actor.

use ratatui::text::Span;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A unique identifier for a [`ChatEntry`].
///
/// Auto-generated as a UUID. Used by prompt assembly strategies
/// to reference specific entries without positional coupling.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChatEntryId(uuid::Uuid);

impl ChatEntryId {
    /// Generate a new unique ID.
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    /// Returns the underlying UUID value.
    #[must_use]
    pub fn as_uuid(&self) -> &uuid::Uuid {
        &self.0
    }
}

impl Default for ChatEntryId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ChatEntryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Where a pinned entry should appear in the assembled prompt.
///
/// Entries with a pin position are never discarded by prompt assembly strategies
/// (sliding window, token budget, compaction). The position controls *where*
/// they appear in the final assembled prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PinPosition {
    /// Always appear at the very beginning of the assembled prompt.
    Top,
    /// Always appear just before the most recent message.
    Bottom,
    /// Stay at this entry's original position in history.
    Relative,
}

impl std::fmt::Display for PinPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Top => write!(f, "TOP"),
            Self::Bottom => write!(f, "BOTTOM"),
            Self::Relative => write!(f, "RELATIVE"),
        }
    }
}

/// Structured table data for rendering in the chat log.
///
/// Carries styled headers and rows so the renderer can produce
/// a properly aligned table with per-cell coloring.
/// This type is not serializable — table entries are ephemeral
/// and do not survive session persistence.
#[derive(Debug, Clone)]
pub struct TableData {
    /// Column headers (styled).
    pub headers: Vec<Span<'static>>,
    /// Data rows, one `Vec<Span>` per row (one span per column).
    pub rows: Vec<Vec<Span<'static>>>,
}

impl TableData {
    /// Returns a plain-text representation of the table for serialization fallback.
    pub(crate) fn to_plain_text(&self) -> String {
        let mut lines = Vec::new();
        let header_text: Vec<&str> = self.headers.iter().map(|s| s.content.as_ref()).collect();
        lines.push(header_text.join(" | "));
        for row in &self.rows {
            let cell_text: Vec<&str> = row.iter().map(|s| s.content.as_ref()).collect();
            lines.push(cell_text.join(" | "));
        }
        lines.join("\n")
    }
}

/// A single entry in the chat history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatEntry {
    /// Unique identifier for this entry.
    pub id: ChatEntryId,
    /// When this entry was created.
    pub timestamp: jiff::Timestamp,
    /// What kind of entry this is.
    pub kind: ChatEntryKind,
    /// Whether this entry is pinned to the context, and where.
    ///
    /// Pinned entries are never discarded by prompt assembly strategies.
    /// `None` (default) means the entry is not pinned.
    ///
    /// OWNER: context-actor (individual mutations via PinChatEntry/UnpinChatEntry),
    ///        session-actor (atomic bulk restore during SessionLoadCompleted via restore_history).
    #[serde(default)]
    pub pin_position: Option<PinPosition>,
}

/// The kind of chat entry.
///
/// # Serialization
///
/// The `Table` variant is not serializable (it contains ratatui `Span`s).
/// Custom `Serialize`/`Deserialize` impls handle this: `Table` serializes
/// as `System` with a plain-text fallback, and is never produced during
/// deserialization (tables are ephemeral display data).
#[derive(Debug, Clone, PartialEq)]
pub enum ChatEntryKind {
    /// A message typed by the user.
    User(String),
    /// A system-generated message (status updates, etc.).
    System(String),
    /// An error message displayed prominently (e.g., stream cancelled).
    Error(String),
    /// A response from an AI assistant.
    Assistant(String),
    /// A message from an actor, identified by source name.
    Actor {
        /// The name of the actor that produced this entry.
        source: String,
        /// The message text.
        text: String,
    },
    /// A structured table with styled headers and rows.
    Table(TableData),
    /// A tool call requested by the LLM.
    ToolCall {
        /// Unique ID assigned by the LLM provider.
        id: String,
        /// The function name.
        name: String,
        /// The JSON arguments string.
        arguments: String,
    },
    /// The result of executing a tool call.
    ToolResult {
        /// The ID of the tool call this result is for.
        id: String,
        /// The function name.
        name: String,
        /// The output content.
        content: String,
        /// Whether execution succeeded.
        success: bool,
    },
}

impl ChatEntry {
    /// Create a new user chat entry with the current timestamp.
    #[must_use]
    pub fn user<T>(text: T) -> Self
    where
        T: Into<String>,
    {
        Self {
            id: ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::User(text.into()),
            pin_position: None,
        }
    }

    /// Create a new system chat entry with the current timestamp.
    #[must_use]
    pub fn system<T>(text: T) -> Self
    where
        T: Into<String>,
    {
        Self {
            id: ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::System(text.into()),
            pin_position: None,
        }
    }

    /// Create a new error chat entry with the current timestamp.
    #[must_use]
    pub fn error<T>(text: T) -> Self
    where
        T: Into<String>,
    {
        Self {
            id: ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::Error(text.into()),
            pin_position: None,
        }
    }

    /// Create a new assistant chat entry with the current timestamp.
    #[must_use]
    pub fn assistant<T>(text: T) -> Self
    where
        T: Into<String>,
    {
        Self {
            id: ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::Assistant(text.into()),
            pin_position: None,
        }
    }

    /// Create a new actor chat entry with the current timestamp.
    #[must_use]
    pub fn actor<S, T>(source: S, text: T) -> Self
    where
        S: Into<String>,
        T: Into<String>,
    {
        Self {
            id: ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::Actor {
                source: source.into(),
                text: text.into(),
            },
            pin_position: None,
        }
    }

    /// Create a new table entry with the current timestamp.
    #[must_use]
    pub fn table(data: TableData) -> Self {
        Self {
            id: ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::Table(data),
            pin_position: None,
        }
    }

    /// Create a new tool call entry with the current timestamp.
    #[must_use]
    pub fn tool_call<S1, S2, S3>(id: S1, name: S2, arguments: S3) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
        S3: Into<String>,
    {
        Self {
            id: ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::ToolCall {
                id: id.into(),
                name: name.into(),
                arguments: arguments.into(),
            },
            pin_position: None,
        }
    }

    /// Create a new tool result entry with the current timestamp.
    #[must_use]
    pub fn tool_result<S1, S2, S3>(id: S1, name: S2, content: S3, success: bool) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
        S3: Into<String>,
    {
        Self {
            id: ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::ToolResult {
                id: id.into(),
                name: name.into(),
                content: content.into(),
                success,
            },
            pin_position: None,
        }
    }

    /// Set the pin position on this entry, returning the modified entry.
    ///
    /// Used as a builder: `ChatEntry::user("instruction").with_pin(PinPosition::Top)`
    #[must_use]
    pub fn with_pin(mut self, position: PinPosition) -> Self {
        self.pin_position = Some(position);
        self
    }

    /// Whether this entry is pinned to the context.
    pub fn is_pinned(&self) -> bool {
        self.pin_position.is_some()
    }

    /// The pin position, if this entry is pinned.
    pub fn pin_position(&self) -> Option<PinPosition> {
        self.pin_position
    }

    /// Returns a static string identifying the entry kind.
    ///
    /// Used by plugins to identify entry types without matching on the enum.
    #[must_use]
    pub fn kind_str(&self) -> &'static str {
        match self.kind {
            ChatEntryKind::User(..) => "user",
            ChatEntryKind::System(..) => "system",
            ChatEntryKind::Error(..) => "error",
            ChatEntryKind::Assistant(..) => "assistant",
            ChatEntryKind::Actor { .. } => "actor",
            ChatEntryKind::Table(..) => "table",
            ChatEntryKind::ToolCall { .. } => "tool_call",
            ChatEntryKind::ToolResult { .. } => "tool_result",
        }
    }

    /// Returns the text content of this entry.
    ///
    /// Returns the primary text for each variant. For `Table`, returns
    /// the plain-text representation. For `ToolCall` and `ToolResult`,
    /// returns a formatted summary.
    #[must_use]
    pub fn text(&self) -> String {
        match &self.kind {
            ChatEntryKind::User(t)
            | ChatEntryKind::System(t)
            | ChatEntryKind::Error(t)
            | ChatEntryKind::Assistant(t) => t.clone(),
            ChatEntryKind::Actor { text, .. } => text.clone(),
            ChatEntryKind::Table(data) => data.to_plain_text(),
            ChatEntryKind::ToolCall {
                name, arguments, ..
            } => {
                format!("{name}: {arguments}")
            }
            ChatEntryKind::ToolResult { name, content, .. } => {
                format!("{name}: {content}")
            }
        }
    }
}

impl Serialize for ChatEntryKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            // Table serializes as System with a plain-text fallback.
            ChatEntryKind::Table(data) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("System", &data.to_plain_text())?;
                map.end()
            }
            ChatEntryKind::User(t) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("User", t)?;
                map.end()
            }
            ChatEntryKind::System(t) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("System", t)?;
                map.end()
            }
            ChatEntryKind::Error(t) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("Error", t)?;
                map.end()
            }
            ChatEntryKind::Assistant(t) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("Assistant", t)?;
                map.end()
            }
            ChatEntryKind::Actor { source, text } => {
                #[derive(Serialize)]
                struct ActorData {
                    source: String,
                    text: String,
                }
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(
                    "Actor",
                    &ActorData {
                        source: source.clone(),
                        text: text.clone(),
                    },
                )?;
                map.end()
            }
            ChatEntryKind::ToolCall {
                id,
                name,
                arguments,
            } => {
                #[derive(Serialize)]
                struct ToolCallData {
                    id: String,
                    name: String,
                    arguments: String,
                }
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(
                    "ToolCall",
                    &ToolCallData {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: arguments.clone(),
                    },
                )?;
                map.end()
            }
            ChatEntryKind::ToolResult {
                id,
                name,
                content,
                success,
            } => {
                #[derive(Serialize)]
                struct ToolResultData {
                    id: String,
                    name: String,
                    content: String,
                    success: bool,
                }
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(
                    "ToolResult",
                    &ToolResultData {
                        id: id.clone(),
                        name: name.clone(),
                        content: content.clone(),
                        success: *success,
                    },
                )?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for ChatEntryKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{self, MapAccess, Visitor};
        use std::fmt;

        struct ChatEntryKindVisitor;

        impl<'de> Visitor<'de> for ChatEntryKindVisitor {
            type Value = ChatEntryKind;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a ChatEntryKind map")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let key: String = map
                    .next_key()?
                    .ok_or_else(|| de::Error::missing_field("variant"))?;
                match key.as_str() {
                    "User" => {
                        let text: String = map.next_value()?;
                        Ok(ChatEntryKind::User(text))
                    }
                    "System" => {
                        let text: String = map.next_value()?;
                        Ok(ChatEntryKind::System(text))
                    }
                    "Error" => {
                        let text: String = map.next_value()?;
                        Ok(ChatEntryKind::Error(text))
                    }
                    "Assistant" => {
                        let text: String = map.next_value()?;
                        Ok(ChatEntryKind::Assistant(text))
                    }
                    "Actor" => {
                        #[derive(Deserialize)]
                        struct ActorData {
                            source: String,
                            text: String,
                        }
                        let data: ActorData = map.next_value()?;
                        Ok(ChatEntryKind::Actor {
                            source: data.source,
                            text: data.text,
                        })
                    }
                    "ToolCall" => {
                        #[derive(Deserialize)]
                        struct ToolCallData {
                            id: String,
                            name: String,
                            arguments: String,
                        }
                        let data: ToolCallData = map.next_value()?;
                        Ok(ChatEntryKind::ToolCall {
                            id: data.id,
                            name: data.name,
                            arguments: data.arguments,
                        })
                    }
                    "ToolResult" => {
                        #[derive(Deserialize)]
                        struct ToolResultData {
                            id: String,
                            name: String,
                            content: String,
                            success: bool,
                        }
                        let data: ToolResultData = map.next_value()?;
                        Ok(ChatEntryKind::ToolResult {
                            id: data.id,
                            name: data.name,
                            content: data.content,
                            success: data.success,
                        })
                    }
                    // "Table" is never deserialized — it's ephemeral.
                    other => Err(de::Error::unknown_variant(
                        other,
                        &[
                            "User",
                            "System",
                            "Error",
                            "Assistant",
                            "Actor",
                            "ToolCall",
                            "ToolResult",
                        ],
                    )),
                }
            }
        }

        deserializer.deserialize_map(ChatEntryKindVisitor)
    }
}

impl PartialEq for TableData {
    fn eq(&self, other: &Self) -> bool {
        self.to_plain_text() == other.to_plain_text()
    }
}

impl Eq for ChatEntryKind {}

#[cfg(test)]
#[path = "chat_entry_tests.rs"]
mod chat_tests;
