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
// Some fields are private (capsule-sealed); the rest remain `pub` until
// Phase 2 caps their accessors too. Tracked in .plans/seal-capsule-walls/plan.md.
#[expect(
    clippy::partial_pub_fields,
    reason = "remaining pub fields pending Phase 2 capsule sealing"
)]
pub struct ContextAssemblyState {
    /// Discovered personas from `~/.config/jinn/personas/`.
    /// OWNER: context-actor (replaces on PersonasLoaded event).
    personas: Vec<Persona>,
    /// The currently active persona (injected into system prompt).
    /// OWNER: context-actor (updated on PersonasLoaded, set on picker confirm).
    active_persona: Option<Persona>,
    /// Global tool definitions available to all sessions.
    /// OWNER: tools-actor (populated on ToolsRegistered event), read by context-actor and llm-actor.
    pub global_tool_definitions: HashMap<String, ToolDefinition>,

    /// Per-session tool definition overrides, keyed by session id.
    /// OWNER: tools-actor (populated on ToolsRegistered event with session_id).
    pub session_tool_definitions:
        HashMap<crate::protocol::SessionId, HashMap<String, ToolDefinition>>,

    /// Loaded compaction system prompt from `~/.config/jinn/prompts/_compaction.md`.
    /// OWNER: populated once at startup by the app init code.
    pub compaction_prompt: String,
}

impl ContextAssemblyState {
    /// Returns all tool definitions available to a session:
    /// global tools + that session's per-session tool overrides.
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

    /// Append a persona to the catalog.
    pub fn push_persona(&mut self, persona: Persona) {
        self.personas.push(persona);
    }
}

impl ContextAssemblyState {
    /// The currently active persona, if any.
    pub fn active_persona(&self) -> Option<&Persona> {
        self.active_persona.as_ref()
    }

    /// The full catalog of available personas.
    pub fn personas(&self) -> &Vec<Persona> {
        &self.personas
    }
}
