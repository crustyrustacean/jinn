//! Conversation data model for the chat log.
//!
//! Each [`ChatEntry`] records a timestamped message from the user,
//! the system, or an actor.

use ratatui::text::{Line, Span};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::feat::session::tool_result_status::ToolResultStatus;

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
        Self(uuid::Uuid::now_v7())
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

impl From<String> for ChatEntryId {
    fn from(s: String) -> Self {
        Self(uuid::Uuid::parse_str(&s).unwrap_or_else(|_| uuid::Uuid::now_v7()))
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

/// Extract plain text from a list of styled `Line`s.
///
/// Joins each line's span content with `\n` separators.
/// Used by `text()`, `content_fingerprint()`, and serialization fallback.
fn lines_to_plain_text(lines: &[Line<'static>]) -> String {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
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
    /// Whether this entry has been compacted and should be skipped during
    /// prompt assembly.
    ///
    /// When `true`, the entry is excluded from all prompt assembly strategies
    /// (sliding window, token budget, compaction) and from token estimation.
    /// Pinned entries override this: if an entry is both pinned and ignored,
    /// it is still included in prompt assembly.
    ///
    /// OWNER: compaction-actor (sets to `true` during compaction).
    #[serde(default)]
    pub ignored: bool,
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
    ///
    /// `display` is what the user typed (shown in the UI, used for session titles).
    /// `expanded` is the token-expanded text (sent to the LLM).
    /// When no prompt tokens are present, both fields are identical.
    User {
        /// What the user typed (shown in UI, used for session titles).
        display: String,
        /// Token-expanded text (sent to the LLM).
        expanded: String,
    },
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
    /// Reasoning/thinking content extracted from the LLM response.
    ///
    /// Displayed in the chat log but excluded from context assembly.
    /// Models like DeepSeek-R1 and Qwen3 embed reasoning in `<think>` tags;
    /// the `reasoning-parser` crate extracts it during streaming.
    Thinking(String),
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
        /// Execution status (pending, success, or failure).
        status: ToolResultStatus,
        /// Full untruncated content, if truncation occurred.
        full_content: Option<String>,
        /// Truncation metadata, if truncation occurred.
        truncation: Option<nullslop_provider::tool_types::TruncationMeta>,
    },
    /// A skill loaded into the session context.
    ///
    /// Created when the `skill` built-in tool loads a SKILL.md file.
    /// Always pinned to TOP. Renders as a collapsible entry showing
    /// only the skill name by default.
    Skill {
        /// The skill name.
        name: String,
        /// Absolute path to the SKILL.md file.
        location: String,
        /// The full skill content (body of SKILL.md with frontmatter stripped).
        content: String,
    },
    /// An informational message (UI-only, not sent to the LLM).
    ///
    /// Used for welcome messages and other user-facing hints.
    /// Excluded from prompt assembly, token estimation, LLM context,
    /// and session persistence. Cannot be pinned.
    ///
    /// Contains pre-built styled lines for rich formatting.
    /// Falls back to plain text during serialization.
    Info(Vec<Line<'static>>),
    /// A compaction summary entry created by the compaction actor.
    ///
    /// Contains a structured summary of previous conversation history
    /// generated by an LLM. Acts as a delimiter in the chat history —
    /// entries before a compaction that are not pinned or system messages
    /// are marked `ignored` and skipped during prompt assembly.
    ///
    /// Multiple compactions are carried forward as-is; each new compaction
    /// only summarizes entries between the previous compaction and now.
    Compaction {
        /// The LLM-generated structured summary text.
        summary: String,
        /// Estimated tokens in the compacted region before compaction.
        tokens_before: usize,
        /// How many entries were compacted (marked as ignored).
        entries_compacted: usize,
        /// Which model was used to generate the summary.
        model_used: String,
    },
}

impl ChatEntry {
    /// Create a new user chat entry with the current timestamp.
    #[must_use]
    pub fn user<T>(text: T) -> Self
    where
        T: Into<String>,
    {
        let t = text.into();
        Self {
            id: ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::User {
                display: t.clone(),
                expanded: t,
            },
            pin_position: None,
            ignored: false,
        }
    }

    /// Create a user entry with separate display and expanded text.
    ///
    /// Use when prompt token expansion produces a different expanded text
    /// than what the user typed.
    #[must_use]
    pub fn user_expanded<D, E>(display: D, expanded: E) -> Self
    where
        D: Into<String>,
        E: Into<String>,
    {
        Self {
            id: ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::User {
                display: display.into(),
                expanded: expanded.into(),
            },
            pin_position: None,
            ignored: false,
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
            ignored: false,
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
            ignored: false,
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
            ignored: false,
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
            ignored: false,
        }
    }

    /// Create a new thinking entry with the current timestamp.
    #[must_use]
    pub fn thinking<T>(text: T) -> Self
    where
        T: Into<String>,
    {
        Self {
            id: ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::Thinking(text.into()),
            pin_position: None,
            ignored: false,
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
            ignored: false,
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
            ignored: false,
        }
    }

    /// Create a new tool result entry with the current timestamp.
    #[must_use]
    pub fn tool_result<S1, S2, S3>(id: S1, name: S2, content: S3, status: ToolResultStatus) -> Self
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
                status,
                full_content: None,
                truncation: None,
            },
            pin_position: None,
            ignored: false,
        }
    }

    /// Create a new tool result entry with truncation metadata.
    #[must_use]
    pub fn tool_result_truncated<S1, S2>(
        id: S1,
        name: S2,
        content: String,
        full_content: String,
        status: ToolResultStatus,
        truncation: nullslop_provider::tool_types::TruncationMeta,
    ) -> Self
    where
        S1: Into<String>,
        S2: Into<String>,
    {
        Self {
            id: ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::ToolResult {
                id: id.into(),
                name: name.into(),
                content,
                status,
                full_content: Some(full_content),
                truncation: Some(truncation),
            },
            pin_position: None,
            ignored: false,
        }
    }

    /// Create a new skill entry with the current timestamp.
    #[must_use]
    pub fn skill<N, L, C>(name: N, location: L, content: C) -> Self
    where
        N: Into<String>,
        L: Into<String>,
        C: Into<String>,
    {
        Self {
            id: ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::Skill {
                name: name.into(),
                location: location.into(),
                content: content.into(),
            },
            pin_position: None,
            ignored: false,
        }
    }

    /// Create a new info chat entry with the current timestamp.
    ///
    /// Info entries are UI-only — they are excluded from prompt assembly,
    /// token estimation, and LLM context. They cannot be pinned and
    /// are not persisted.
    ///
    /// Accepts pre-built styled lines for rich formatting.
    #[must_use]
    pub fn info(lines: Vec<Line<'static>>) -> Self {
        Self {
            id: ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::Info(lines),
            pin_position: None,
            ignored: false,
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

    /// Set the ignored flag on this entry, returning the modified entry.
    ///
    /// Used as a builder: `ChatEntry::user("hello").with_ignored(true)`
    #[must_use]
    pub fn with_ignored(mut self, ignored: bool) -> Self {
        self.ignored = ignored;
        self
    }

    /// Whether this entry is pinned to the context.
    pub fn is_pinned(&self) -> bool {
        self.pin_position.is_some()
    }

    /// Whether this entry kind can be pinned to the context.
    ///
    /// Only user messages, assistant responses, tool results, and skill loads
    /// can be pinned. Info entries, system messages, errors, actors, thinking,
    /// tool calls, tables, and compaction entries are not pinnable.
    #[must_use]
    pub fn is_pinnable(&self) -> bool {
        matches!(
            self.kind,
            ChatEntryKind::User { .. }
                | ChatEntryKind::Assistant(..)
                | ChatEntryKind::ToolResult { .. }
                | ChatEntryKind::Skill { .. }
        )
    }

    /// Whether this entry is a compaction summary.
    ///
    /// Compaction entries act as delimiters in the chat history. They are
    /// never included in subsequent compaction ranges.
    #[must_use]
    pub fn is_compaction(&self) -> bool {
        matches!(self.kind, ChatEntryKind::Compaction { .. })
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
            ChatEntryKind::User { .. } => "user",
            ChatEntryKind::System(..) => "system",
            ChatEntryKind::Error(..) => "error",
            ChatEntryKind::Assistant(..) => "assistant",
            ChatEntryKind::Actor { .. } => "actor",
            ChatEntryKind::Table(..) => "table",
            ChatEntryKind::Thinking(..) => "thinking",
            ChatEntryKind::ToolCall { .. } => "tool_call",
            ChatEntryKind::ToolResult { .. } => "tool_result",
            ChatEntryKind::Skill { .. } => "skill",
            ChatEntryKind::Info(..) => "info",
            ChatEntryKind::Compaction { .. } => "compaction",
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
            ChatEntryKind::User { display, .. } => display.clone(),
            ChatEntryKind::System(t)
            | ChatEntryKind::Error(t)
            | ChatEntryKind::Assistant(t)
            | ChatEntryKind::Thinking(t) => t.clone(),
            ChatEntryKind::Info(lines) => lines_to_plain_text(lines),
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
            ChatEntryKind::Skill { content, .. } => content.clone(),
            ChatEntryKind::Compaction { summary, .. } => summary.clone(),
        }
    }

    /// Compute a cheap fingerprint of this entry's visual content.
    ///
    /// Two entries with the same visual content produce the same fingerprint.
    /// Used by the line count cache to detect content changes without re-rendering.
    /// The fingerprint changes when text content or entry kind changes.
    #[must_use]
    pub fn content_fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        // Hash the kind discriminant + content.
        std::mem::discriminant(&self.kind).hash(&mut hasher);
        match &self.kind {
            ChatEntryKind::User { display, .. } => display.hash(&mut hasher),
            ChatEntryKind::System(t)
            | ChatEntryKind::Error(t)
            | ChatEntryKind::Assistant(t)
            | ChatEntryKind::Thinking(t) => t.hash(&mut hasher),
            ChatEntryKind::Info(lines) => lines_to_plain_text(lines).hash(&mut hasher),
            ChatEntryKind::Actor { text, .. } => text.hash(&mut hasher),
            ChatEntryKind::Table(data) => data.to_plain_text().hash(&mut hasher),
            ChatEntryKind::ToolCall {
                name, arguments, ..
            } => {
                name.hash(&mut hasher);
                arguments.hash(&mut hasher);
            }
            ChatEntryKind::ToolResult {
                name,
                content,
                status,
                truncation,
                ..
            } => {
                name.hash(&mut hasher);
                status.hash(&mut hasher);
                content.hash(&mut hasher);
                // Include truncation presence so the line count cache
                // invalidates when the indicator line is added or removed.
                truncation.is_some().hash(&mut hasher);
            }
            ChatEntryKind::Skill { name, content, .. } => {
                name.hash(&mut hasher);
                content.hash(&mut hasher);
            }
            ChatEntryKind::Compaction {
                summary,
                entries_compacted,
                model_used,
                ..
            } => {
                summary.hash(&mut hasher);
                entries_compacted.hash(&mut hasher);
                model_used.hash(&mut hasher);
            }
        }
        hasher.finish()
    }
}

impl Serialize for ChatEntryKind {
    #[allow(clippy::too_many_lines)]
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            // Table serializes as System with a plain-text fallback.
            ChatEntryKind::Table(data) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("System", &data.to_plain_text())?;
                map.end()
            }
            ChatEntryKind::User { display, expanded } => {
                #[derive(Serialize)]
                struct UserData {
                    display: String,
                    expanded: String,
                }
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(
                    "User",
                    &UserData {
                        display: display.clone(),
                        expanded: expanded.clone(),
                    },
                )?;
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
                status,
                full_content,
                truncation,
            } => {
                #[derive(Serialize)]
                struct ToolResultData {
                    id: String,
                    name: String,
                    content: String,
                    status: ToolResultStatus,
                    #[serde(skip_serializing_if = "Option::is_none")]
                    full_content: Option<String>,
                    #[serde(skip_serializing_if = "Option::is_none")]
                    truncation: Option<nullslop_provider::tool_types::TruncationMeta>,
                }
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(
                    "ToolResult",
                    &ToolResultData {
                        id: id.clone(),
                        name: name.clone(),
                        content: content.clone(),
                        status: *status,
                        full_content: full_content.clone(),
                        truncation: truncation.clone(),
                    },
                )?;
                map.end()
            }
            ChatEntryKind::Skill {
                name,
                location,
                content,
            } => {
                #[derive(Serialize)]
                struct SkillData {
                    name: String,
                    location: String,
                    content: String,
                }
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(
                    "Skill",
                    &SkillData {
                        name: name.clone(),
                        location: location.clone(),
                        content: content.clone(),
                    },
                )?;
                map.end()
            }
            ChatEntryKind::Thinking(t) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("Thinking", t)?;
                map.end()
            }
            ChatEntryKind::Info(lines) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("Info", &lines_to_plain_text(lines))?;
                map.end()
            }
            ChatEntryKind::Compaction {
                summary,
                tokens_before,
                entries_compacted,
                model_used,
            } => {
                #[derive(Serialize)]
                struct CompactionData {
                    summary: String,
                    tokens_before: usize,
                    entries_compacted: usize,
                    model_used: String,
                }
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(
                    "Compaction",
                    &CompactionData {
                        summary: summary.clone(),
                        tokens_before: *tokens_before,
                        entries_compacted: *entries_compacted,
                        model_used: model_used.clone(),
                    },
                )?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for ChatEntryKind {
    #[allow(clippy::too_many_lines)]
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{self, MapAccess, Visitor};
        use std::fmt;

        struct ChatEntryKindVisitor;

        impl<'de> Visitor<'de> for ChatEntryKindVisitor {
            type Value = ChatEntryKind;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a ChatEntryKind map")
            }

            #[allow(clippy::too_many_lines)]
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let key: String = map
                    .next_key()?
                    .ok_or_else(|| de::Error::missing_field("variant"))?;
                match key.as_str() {
                    "User" => {
                        #[derive(Deserialize)]
                        struct UserData {
                            display: String,
                            expanded: String,
                        }
                        let data: UserData = map.next_value()?;
                        Ok(ChatEntryKind::User {
                            display: data.display,
                            expanded: data.expanded,
                        })
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
                        // Supports both new format (status: ToolResultStatus + truncation)
                        // and old format (success: bool) for backward compat.
                        #[derive(Deserialize)]
                        struct ToolResultDataNew {
                            id: String,
                            name: String,
                            content: String,
                            status: ToolResultStatus,
                            #[serde(default)]
                            full_content: Option<String>,
                            #[serde(default)]
                            truncation: Option<nullslop_provider::tool_types::TruncationMeta>,
                        }
                        #[derive(Deserialize)]
                        struct ToolResultDataOld {
                            id: String,
                            name: String,
                            content: String,
                            success: bool,
                        }
                        // Try new format first, fall back to old format.
                        let value: serde_json::Value = map.next_value()?;
                        let result = serde_json::from_value::<ToolResultDataNew>(value.clone())
                            .map(|data| ChatEntryKind::ToolResult {
                                id: data.id,
                                name: data.name,
                                content: data.content,
                                status: data.status,
                                full_content: data.full_content,
                                truncation: data.truncation,
                            })
                            .or_else(|_| {
                                serde_json::from_value::<ToolResultDataOld>(value).map(|data| {
                                    ChatEntryKind::ToolResult {
                                        id: data.id,
                                        name: data.name,
                                        content: data.content,
                                        status: if data.success {
                                            ToolResultStatus::Success
                                        } else {
                                            ToolResultStatus::Failure
                                        },
                                        full_content: None,
                                        truncation: None,
                                    }
                                })
                            })
                            .map_err(|e| {
                                de::Error::custom(format!("failed to deserialize ToolResult: {e}"))
                            })?;
                        Ok(result)
                    }
                    "Skill" => {
                        #[derive(Deserialize)]
                        struct SkillData {
                            name: String,
                            location: String,
                            content: String,
                        }
                        let data: SkillData = map.next_value()?;
                        Ok(ChatEntryKind::Skill {
                            name: data.name,
                            location: data.location,
                            content: data.content,
                        })
                    }
                    "Thinking" => {
                        let text: String = map.next_value()?;
                        Ok(ChatEntryKind::Thinking(text))
                    }
                    "Info" => {
                        // Info entries are not persisted. If we encounter one in
                        // deserialized data (e.g. from an older version), treat
                        // it as System so we don't lose the text.
                        let text: String = map.next_value()?;
                        Ok(ChatEntryKind::System(text))
                    }
                    "Compaction" => {
                        #[derive(Deserialize)]
                        struct CompactionData {
                            summary: String,
                            tokens_before: usize,
                            entries_compacted: usize,
                            model_used: String,
                        }
                        let data: CompactionData = map.next_value()?;
                        Ok(ChatEntryKind::Compaction {
                            summary: data.summary,
                            tokens_before: data.tokens_before,
                            entries_compacted: data.entries_compacted,
                            model_used: data.model_used,
                        })
                    }
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
                            "Thinking",
                            "Skill",
                            "Info",
                            "Compaction",
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
