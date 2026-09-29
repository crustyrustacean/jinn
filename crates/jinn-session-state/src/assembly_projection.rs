//! Coherent projection of the session-owned fields required for prompt assembly.

use std::collections::HashSet;
use std::path::PathBuf;

use jinn_context::ContextFile;
use jinn_core_types::{ChatEntry, SessionId};
use jinn_skills_msg::Skill;

use crate::ChatSessionState;

/// Immutable session values required by the context-assembly service.
///
/// The projection is captured under the application's read guard so every
/// session-owned assembly field comes from one authoritative state version.
/// Persona and tool definitions are not session-owned and are resolved by the
/// context-assembly slice when it builds the final crossing input.
#[derive(Debug, Clone)]
pub struct AssemblySessionProjection {
    /// Session identity.
    pub session_id: SessionId,
    /// Working directory rendered into the environment block.
    pub cwd: PathBuf,
    /// Selected persona name before persona-cell resolution.
    pub persona_name: String,
    /// Full conversation history, including pins.
    pub history: Vec<ChatEntry>,
    /// Tools disabled by session policy.
    pub disabled_tools: HashSet<String>,
    /// Provider selected by the session profile.
    pub provider_name: String,
    /// Skills discovered for the session.
    pub skills: Vec<Skill>,
    /// Skills disabled by session policy.
    pub disabled_skills: HashSet<String>,
    /// Skills whose bodies are loaded.
    pub loaded_skills: HashSet<String>,
    /// Project context files discovered for the session.
    pub context_files: Vec<ContextFile>,

    /// Whether this session is an attendant, which decides whether it is
    /// offered the attendant-only tools.
    pub is_attendant: bool,
}

impl AssemblySessionProjection {
    /// Captures all session-owned assembly fields from the authoritative aggregate.
    #[must_use]
    pub fn capture(session_id: &SessionId, session: &ChatSessionState) -> Self {
        Self {
            session_id: session_id.clone(),
            cwd: session.cwd().to_path_buf(),
            persona_name: session.persona_name().to_owned(),
            history: session.history().to_vec(),
            disabled_tools: session.disabled_tools().clone(),
            provider_name: session.model_selection().provider_name().to_owned(),
            skills: session.discovered_skills().to_vec(),
            disabled_skills: session.disabled_skills().clone(),
            loaded_skills: session.loaded_skills(),
            context_files: session.discovered_context_files().to_vec(),
            is_attendant: session.is_attendant(),
        }
    }
}
