//! MCP server picker intent handlers.
//!
//! The MCP picker lists every server declared in `jinn.toml` under
//! `[[mcp_servers]]` and lets the user toggle which ones are enabled for the
//! active session. Enablement is an opt-in set persisted in `SessionCore`;
//! only the toggled-on servers spawn an `McpActor` (Phase 4 wiring).
//!
//! Like the tool/skill pickers, toggles mutate the picker entries in memory and
//! a snapshot of the pre-edit enabled set is taken on open so ESC can revert.
//! [`handle_picker_confirm`](crate::feat::picker::intent::handle_picker_confirm)
//! commits the final set via [`confirm_mcp`].

use std::collections::BTreeSet;

use crate::common::app_state::AppState;
use crate::feat::intent::IntentResult;
use crate::feat::mcp::picker_entry::{McpPreviewMode, McpServerEntry};
use crate::feat::picker::geometry::active_viewport;
use crate::feat::ui::picker_states::PickerExt;

/// Populates the MCP server picker entries from configured servers.
///
/// Reads `state.frontend.preferences.mcp_servers` and marks each entry enabled
/// according to the active session's `enabled_mcp_servers` set.
pub(crate) fn load_mcp_picker_entries(state: &mut AppState) {
    let enabled = state.active_session().enabled_mcp_servers().clone();
    let theme = state.frontend.theme.clone();

    let mut entries: Vec<McpServerEntry> = state
        .frontend
        .preferences
        .mcp_servers
        .iter()
        .map(|server| {
            let name = server.name.clone();
            let description = server.description_for_picker();
            McpServerEntry {
                name: name.clone(),
                description,
                search_text: name.clone(),
                enabled: enabled.contains(&name),
                theme: theme.clone(),
                status: None,
                stderr_tail: String::new(),
                tools: Vec::new(),
                preview_mode: McpPreviewMode::default(),
            }
        })
        .collect();

    entries.sort_by_key(|e| e.name.to_lowercase());

    state.frontend.mcp_server_picker_mut().set_items(entries);
}

/// Toggles the `enabled` state of the currently selected MCP server entry.
///
/// Mutates the picker entry only; the session set is written on confirm.
pub fn handle_mcp_toggle(state: &mut AppState) -> IntentResult {
    state
        .frontend
        .mcp_server_picker_mut()
        .with_selected_mut(|entry| {
            entry.enabled = !entry.enabled;
        });
    let viewport = active_viewport(state);
    state.frontend.mcp_server_picker_mut().move_down(viewport);
    IntentResult::empty()
}

/// Confirms the MCP server picker: collects enabled server names from the
/// picker entries and writes them to the active session's enabled set.
///
/// The session set is the source of truth for which `McpActor`s should be
/// running. Spawning/killing the actors is wired in Phase 4 (lifecycle
/// wiring); this handler only persists the enablement decision and pops the
/// picker scope.
pub(crate) fn confirm_mcp(state: &mut AppState) -> IntentResult {
    let session_id = state.active_session().session_id().clone();
    let enabled: BTreeSet<String> = state
        .frontend
        .mcp_server_picker()
        .items()
        .iter()
        .filter(|entry| entry.enabled)
        .map(|entry| entry.name.clone())
        .collect();

    state
        .active_session_mut()
        .set_enabled_mcp_servers(enabled.clone());
    *state.frontend.mcp_server_picker_snapshot_mut() = None;
    state.frontend.scope_stack.pop();

    // Signal the MCP lifecycle actor to spawn/kill `McpActor`s for the diff
    // between this desired set and the currently-running ones.
    IntentResult::with_message(
        crate::feat::mcp_coordinator_actor::protocol::McpEnablementChanged {
            session_id,
            enabled,
        },
    )
}
