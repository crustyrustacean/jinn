//! Context assembly state — owned by the context-actor.

use std::collections::HashMap;

use crate::feat::context::env_context::ContextFile;
use crate::feat::context::prompt_template::PromptTemplateStore;
use crate::feat::judge::Judge;
use crate::feat::persona::Persona;
use crate::feat::skills::Skill;
use crate::protocol::ToolDefinition;

/// Context assembly state — owned by the context-actor.
///
/// Written to exclusively by `SessionActor` and `IntentHandler`.
/// No other actor should mutate these fields.
#[derive(Debug)]
pub struct ContextAssemblyState {
    /// Loaded prompt templates from `~/.config/jinn/prompts/`.
    /// OWNER: context-actor (replaces on PromptTemplatesLoaded event).
    pub prompt_templates: PromptTemplateStore,

    /// Discovered agent skills from `~/.agents/skills/`.
    /// OWNER: skills-scan-actor (replaces on ScanSkills command).
    pub skills: Vec<Skill>,
    /// Discovered personas from `~/.config/jinn/personas/`.
    /// OWNER: context-actor (replaces on PersonasLoaded event).
    pub personas: Vec<Persona>,
    /// The currently active persona (injected into system prompt).
    /// OWNER: context-actor (updated on PersonasLoaded, set on picker confirm).
    pub active_persona: Option<Persona>,
    /// Registered tool definitions, keyed by tool name.
    /// OWNER: tools-actor (populated on ToolsRegistered event), read by context-actor and llm-actor.
    pub tool_definitions: HashMap<String, ToolDefinition>,
    /// Cached project context files (AGENTS.md, CLAUDE.md).
    /// OWNER: populated on startup, refreshed on session/CWD change.
    pub context_files: Vec<ContextFile>,
    /// Discovered judges from `~/.config/jinn/judges/`.
    /// OWNER: session-actor (replaces on JudgesLoaded event).
    pub judges: Vec<Judge>,
    /// Loaded compaction system prompt from `~/.config/jinn/prompts/_compaction.md`.
    /// OWNER: populated once at startup by the app init code.
    pub compaction_prompt: String,
}

impl Default for ContextAssemblyState {
    fn default() -> Self {
        Self {
            prompt_templates: PromptTemplateStore::new(),
            skills: Vec::new(),
            personas: Vec::new(),
            active_persona: None,
            tool_definitions: HashMap::new(),
            context_files: Vec::new(),
            judges: Vec::new(),
            compaction_prompt: String::new(),
        }
    }
}
