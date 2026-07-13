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
    /// Global tool definitions available to all sessions.
    /// OWNER: tools-actor (populated on ToolsRegistered event), read by context-actor and llm-actor.
    pub global_tool_definitions: HashMap<String, ToolDefinition>,

    /// Per-session tool definitions for attached plugin tools.
    /// OWNER: tools-actor (populated on ToolsRegistered event with session_id).
    pub session_tool_definitions:
        HashMap<crate::protocol::SessionId, HashMap<String, ToolDefinition>>,

    /// Catalog of attachable plugin tool definitions, keyed by tool name.
    /// OWNER: populated once at startup by `actor_wiring.rs` from attachable
    /// tool metadata (the same definitions registered `execution_only: true`
    /// in the tools actor). This is NOT a visibility source — `tools_for_session`
    /// never consults it. It is a private lookup table for
    /// `DomainNodeContext::create_child_session`, which copies selected entries
    /// into `session_tool_definitions[child]` when a plugin spawns a child
    /// session. Keeping the origin's tool catalog clean.
    pub attachable_tool_catalog: HashMap<String, ToolDefinition>,

    /// Loaded compaction system prompt from `~/.config/jinn/prompts/_compaction.md`.
    /// OWNER: populated once at startup by the app init code.
    pub compaction_prompt: String,
}

impl ContextAssemblyState {
    /// Returns all tool definitions available to a session:
    /// global tools + that session's attached plugin tools.
    /// Session tools override global tools of the same name.
    pub fn tools_for_session(
        &self,
        session_id: &crate::protocol::SessionId,
    ) -> Vec<ToolDefinition> {
        let mut tools: Vec<ToolDefinition> =
            self.global_tool_definitions.values().cloned().collect();
        if let Some(session_tools) = self.session_tool_definitions.get(session_id) {
            let global_names: std::collections::HashSet<String> =
                self.global_tool_definitions.keys().cloned().collect();
            for (name, def) in session_tools {
                if global_names.contains(name) {
                    if let Some(pos) = tools.iter().position(|t| &t.name == name)
                        && let Some(slot) = tools.get_mut(pos)
                    {
                        *slot = def.clone();
                    }
                } else {
                    tools.push(def.clone());
                }
            }
        }
        tools
    }

    /// Set the active persona.
    pub fn set_active_persona(&mut self, persona: Option<Persona>) {
        self.active_persona = persona;
    }

    /// Replace the full persona list.
    pub fn set_personas(&mut self, personas: Vec<Persona>) {
        self.personas = personas;
    }
}
