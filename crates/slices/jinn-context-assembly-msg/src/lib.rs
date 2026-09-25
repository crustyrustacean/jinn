//! Shared contracts for the context-assembly slice.
//!
//! This crate defines the inputs, requests, and responses exchanged across
//! the context-assembly boundary without depending on its implementation.

use std::collections::HashSet;
use std::path::PathBuf;

use jinn_context::ContextFile;
use jinn_core_types::{ChatEntry, SessionId, ToolDefinition};
use jinn_persona_msg::Persona;
use jinn_skills_msg::Skill;
use jinn_slices::AssembledPrompt;
use serde::{Deserialize, Serialize};

/// Everything assembly needs, provided by the caller.
///
/// The context-assembly service never reads `AppState`: whoever
/// dispatches a turn snapshots the session state it can see into this
/// struct and sends it with the [`AssembleContext`] message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssemblyInputs {
    /// The session being assembled for.
    pub session_id: SessionId,
    /// The session's working directory (rendered into the env block).
    pub cwd: PathBuf,
    /// The resolved persona payload (`None` renders no persona section).
    pub persona: Option<Persona>,
    /// The session's full history (pins included; partitioned here).
    pub history: Vec<ChatEntry>,
    /// Merged (global + session-override) tool definitions, unfiltered.
    pub tools: Vec<ToolDefinition>,
    /// Tool names the session disabled.
    pub disabled_tools: HashSet<String>,
    /// The provider the request will go to (for server-tool filtering).
    pub provider_name: String,
    /// Discovered skills, unfiltered.
    pub skills: Vec<Skill>,
    /// Skill names the session disabled.
    pub disabled_skills: HashSet<String>,
    /// Skill names whose bodies are loaded.
    pub loaded_skills: HashSet<String>,
    /// Discovered context files (rendered into the env block).
    pub context_files: Vec<ContextFile>,
}

/// Ask message: assemble a prompt from these inputs.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Assemble a system prompt + messages from caller-provided inputs.")]
pub struct AssembleContext {
    pub inputs: AssemblyInputs,
}

/// Reply: the assembled prompt (deserialized from the trouper reply
/// payload). `AssembledPrompt` is the shared type in `jinn-slices`.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "The assembled prompt reply from the context-assembly service.")]
pub struct AssembledResponse {
    pub session_id: SessionId,
    pub prompt: AssembledPrompt,
}

/// Emitted when a chat entry's context override is toggled (e.g. via the `x` keybind).
///
/// The intent handler emits this after toggling an entry's inclusion in
/// the LLM context. The `ContextSizeActor` subscribes to this event to
/// recalculate the context size for the status bar.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A chat entry's context override changed.")]
pub struct ContextOverrideChanged {
    /// The session whose entry was toggled.
    pub session_id: SessionId,
    /// The entry whose context override changed.
    pub entry_id: jinn_core_types::ChatEntryId,
}

impl jinn_slices::BusMessage for ContextOverrideChanged {}
