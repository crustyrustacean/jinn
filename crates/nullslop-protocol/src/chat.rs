//! Conversation data model for the chat log.
//!
//! Each [`ChatEntry`] records a timestamped message from the user,
//! the system, or an actor.

use serde::{Deserialize, Serialize};

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

/// A single entry in the chat history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatEntry {
    /// Unique identifier for this entry.
    pub id: ChatEntryId,
    /// When this entry was created.
    pub timestamp: jiff::Timestamp,
    /// What kind of entry this is.
    pub kind: ChatEntryKind,
}

/// The kind of chat entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatEntryKind {
    /// A message typed by the user.
    User(String),
    /// A system-generated message (status updates, etc.).
    System(String),
    /// A response from an AI assistant.
    Assistant(String),
    /// A message from an actor, identified by source name.
    Actor {
        /// The name of the actor that produced this entry.
        source: String,
        /// The message text.
        text: String,
    },
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
        }
    }
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod chat_tests;
