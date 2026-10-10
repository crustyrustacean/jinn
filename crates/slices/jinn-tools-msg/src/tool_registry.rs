//! The tools family's shared registry payload.
//!
//! The global + per-session tool definition registries were formerly
//! fields on the kernel's `ContextAssemblyState`, then a `jinn-slices`
//! cell; they are genuinely multi-party — the tools orchestrator
//! announces via `ToolsRegistered`, the kernel session actor writes
//! the cell, and the queue/session dispatch snapshots, the TUI, and
//! the MCP picker refresh read it.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use jinn_core_types::SessionId;
use jinn_core_types::ToolDefinition;

use jinn_slices::SlotKey;

/// The tools family's registry payload: global definitions plus
/// per-session overrides (session tools shadow global tools by name).
#[derive(Debug, Default, Clone)]
pub struct ToolRegistry {
    /// Global tools available to all sessions.
    pub global: BTreeMap<String, ToolDefinition>,
    /// Per-session overrides keyed by session id.
    pub session: BTreeMap<SessionId, BTreeMap<String, ToolDefinition>>,
    /// Global providers one session has opted out of, keyed by session id.
    ///
    /// A singleton MCP server registers globally — one shared connection
    /// serves every session — so "this session disabled it" cannot remove
    /// the registration (other sessions still use it). Instead the session
    /// records the provider here, and [`Self::tools_for_session`] withholds
    /// that provider's global tools from the session's view.
    pub session_denied: BTreeMap<SessionId, BTreeSet<String>>,
}

impl ToolRegistry {
    /// Merged tools for one session: global tools with that session's
    /// overrides replacing same-name entries (identical semantics to
    /// the former `ContextAssemblyState::tools_for_session`). Global tools
    /// belonging to a provider the session has denied are withheld.
    #[must_use]
    pub fn tools_for_session(&self, session_id: &SessionId) -> Vec<ToolDefinition> {
        let denied = self.session_denied.get(session_id);
        let mut tools: Vec<ToolDefinition> = self
            .global
            .values()
            .filter(|def| {
                denied.is_none_or(|providers| !providers.iter().any(|p| def.name.starts_with(p)))
            })
            .cloned()
            .collect();
        if let Some(session_tools) = self.session.get(session_id) {
            let global_names: HashSet<String> = self.global.keys().cloned().collect();
            for (name, def) in session_tools {
                if global_names.contains(name)
                    && let Some(pos) = tools.iter().position(|t| &t.name == name)
                    && let Some(slot) = tools.get_mut(pos)
                {
                    *slot = def.clone();
                } else {
                    tools.push(def.clone());
                }
            }
        }
        tools
    }

    /// Records that `session_id` opted out of `provider`'s global tools.
    pub fn deny_provider_for_session(&mut self, session_id: &SessionId, provider: &str) {
        self.session_denied
            .entry(session_id.clone())
            .or_default()
            .insert(provider.to_owned());
    }

    /// Clears a session's opt-out of `provider`'s global tools.
    pub fn admit_provider_for_session(&mut self, session_id: &SessionId, provider: &str) {
        if let Some(providers) = self.session_denied.get_mut(session_id) {
            providers.remove(provider);
            if providers.is_empty() {
                self.session_denied.remove(session_id);
            }
        }
    }

    /// Drops every opt-out recorded for one session.
    pub fn clear_denials_for_session(&mut self, session_id: &SessionId) {
        self.session_denied.remove(session_id);
    }
}

/// The slot key the tools registry cell lives under.
#[must_use]
pub fn tools_registry_slot() -> SlotKey {
    SlotKey::builtin("tools", "registry")
}

#[cfg(test)]
mod registry_tests {
    #![allow(clippy::expect_used, reason = "test code")]

    use super::*;

    fn definition(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_owned(),
            description: String::new(),
            parameters: serde_json::json!({}),
            prompt_snippet: None,
            prompt_guidelines: vec![],
            server_tool_type: None,
        }
    }

    #[rstest::rstest]
    fn denied_provider_hides_only_its_global_tools_for_that_session() {
        // Given a registry with two global providers registered.
        let session_id = SessionId::new();
        let mut registry = ToolRegistry::default();
        registry.global.insert(
            "mcp__bundled-example__create_graph".to_owned(),
            definition("mcp__bundled-example__create_graph"),
        );
        registry.global.insert(
            "mcp__other__tool".to_owned(),
            definition("mcp__other__tool"),
        );

        // When the session denies the bundled-example provider.
        registry.deny_provider_for_session(&session_id, "mcp__bundled-example");
        let tools = registry.tools_for_session(&session_id);

        // Then the bundled-example tool is withheld and the other provider's is not.
        assert!(
            !tools
                .iter()
                .any(|d| d.name == "mcp__bundled-example__create_graph")
        );
        assert!(tools.iter().any(|d| d.name == "mcp__other__tool"));
    }

    #[rstest::rstest]
    fn a_denial_is_scoped_to_its_session() {
        // Given a registry where session A denied a provider.
        let denied_session = SessionId::new();
        let other_session = SessionId::new();
        let mut registry = ToolRegistry::default();
        registry.global.insert(
            "mcp__bundled-example__query".to_owned(),
            definition("mcp__bundled-example__query"),
        );
        registry.deny_provider_for_session(&denied_session, "mcp__bundled-example");

        // When reading the other session's tools.
        let tools = registry.tools_for_session(&other_session);

        // Then the other session still sees the global tool.
        assert!(
            tools
                .iter()
                .any(|d| d.name == "mcp__bundled-example__query")
        );
    }

    #[rstest::rstest]
    fn readmitting_restores_the_global_tools() {
        // Given a session that denied a provider.
        let session_id = SessionId::new();
        let mut registry = ToolRegistry::default();
        registry.global.insert(
            "mcp__bundled-example__transact".to_owned(),
            definition("mcp__bundled-example__transact"),
        );
        registry.deny_provider_for_session(&session_id, "mcp__bundled-example");

        // When the session re-admits the provider.
        registry.admit_provider_for_session(&session_id, "mcp__bundled-example");
        let tools = registry.tools_for_session(&session_id);

        // Then the global tool is visible again.
        assert!(
            tools
                .iter()
                .any(|d| d.name == "mcp__bundled-example__transact")
        );
    }

    #[rstest::rstest]
    fn clearing_denials_restores_every_provider_at_once() {
        // Given a session that denied two providers.
        let session_id = SessionId::new();
        let mut registry = ToolRegistry::default();
        registry
            .global
            .insert("mcp__a__t".to_owned(), definition("mcp__a__t"));
        registry
            .global
            .insert("mcp__b__t".to_owned(), definition("mcp__b__t"));
        registry.deny_provider_for_session(&session_id, "mcp__a");
        registry.deny_provider_for_session(&session_id, "mcp__b");

        // When clearing the session's denials.
        registry.clear_denials_for_session(&session_id);

        // Then both providers' tools are visible again.
        let tools = registry.tools_for_session(&session_id);
        assert_eq!(tools.len(), 2);
    }

    #[rstest::rstest]
    fn denials_leave_session_scoped_tools_untouched() {
        // Given a session with its own session-scoped tool alongside a
        // denied global provider.
        let session_id = SessionId::new();
        let mut registry = ToolRegistry::default();
        registry.global.insert(
            "mcp__bundled-example__query".to_owned(),
            definition("mcp__bundled-example__query"),
        );
        registry
            .session
            .entry(session_id.clone())
            .or_default()
            .insert("my_session_tool".to_owned(), definition("my_session_tool"));

        // When denying the provider and reading the session's tools.
        registry.deny_provider_for_session(&session_id, "mcp__bundled-example");
        let tools = registry.tools_for_session(&session_id);

        // Then the session-scoped tool remains — the denial only withholds
        // global registrations.
        assert!(tools.iter().any(|d| d.name == "my_session_tool"));
        assert!(
            !tools
                .iter()
                .any(|d| d.name == "mcp__bundled-example__query")
        );
    }
}
