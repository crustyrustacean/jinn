//! Context assembly state - owned by the context-actor.

use std::collections::HashMap;

use crate::feat::persona::Persona;
use crate::protocol::ToolDefinition;

/// Context assembly state - owned by the context-actor.
///
/// Written to exclusively by `SessionActor` and `IntentHandler`.
/// No other actor should mutate these fields.
///
/// Note: cwd-scoped discovery (`skills`, `prompt_templates`, `context_files`) lives
/// per-session on `ChatSession` ephemeral state, not here, so concurrent sessions with
/// different cwds never clobber each other.
#[derive(Debug, Default)]
pub struct ContextAssemblyState {
    /// Discovered personas from `~/.config/jinn/personas/`.
    /// OWNER: context-actor (replaces on PersonasLoaded event).
    pub personas: Vec<Persona>,
    /// The currently active persona (injected into system prompt).
    /// OWNER: context-actor (updated on PersonasLoaded, set on picker confirm).
    pub active_persona: Option<Persona>,
    /// Registered tool definitions, keyed by tool name.
    /// OWNER: tools-actor (populated on ToolsRegistered event), read by context-actor and llm-actor.
    pub tool_definitions: HashMap<String, ToolDefinition>,

    /// Loaded compaction system prompt from `~/.config/jinn/prompts/_compaction.md`.
    /// OWNER: populated once at startup by the app init code.
    pub compaction_prompt: String,
}
