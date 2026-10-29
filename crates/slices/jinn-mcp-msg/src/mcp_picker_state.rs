// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! The MCP server inspector's own state.
//!
//! The menu is this cell: rows, filter, highlight, and the pre-open enabled
//! snapshot that `ESC` restores. Nothing here is mirrored into app state, so
//! the actor that loaded the rows and the renderer that draws them cannot end
//! up on different stores.

use crate::McpServerEntry;
use jinn_picker::PickerEntry;
use jinn_slices::{SliceScopeId, SlotKey};

/// The selection list the inspector draws.
pub type McpServerList = jinn_selection_widget::SelectionState<PickerEntry<McpServerEntry>>;

/// The inspector's state: the server list, the pre-open enabled set, and the
/// row count the render pass measured.
#[derive(Debug, Default)]
pub struct McpPickerState {
    /// The server rows, their filter, and the highlight.
    pub selection: McpServerList,
    /// The session's enabled server names as they were when the inspector
    /// opened. `ESC` restores them; a confirm clears them.
    pub snapshot: Option<std::collections::BTreeSet<String>>,
    /// Rows the render pass measured, so the navigation keys page by a real
    /// window rather than a fallback.
    pub results_viewport: usize,
    /// The preview pane's scroll offset. Always zero: this inspector's
    /// preview has no scroll mechanism, so it holds still across cursor
    /// moves rather than jumping back to the top each time.
    pub preview_scroll: usize,
    /// The session the inspector was opened for.
    ///
    /// The render pass has no app state, so the open action parks the active
    /// session's id here and the live refresh reads it back. Status, stderr,
    /// and tools are all per-session, so a stale id would show the wrong
    /// server's health.
    pub session_id: Option<jinn_core_types::SessionId>,
    /// The registered tools of the enabled servers, as the live refresh last
    /// read the registry.
    ///
    /// Kept whole so a signature change can recompute the cost figure without
    /// a second registry read, and so tests can assert against the exact set
    /// the figure was counted from.
    pub context_definitions: Vec<jinn_core_types::ToolDefinition>,
    /// Context cost in tokens of the enabled servers' registered tool
    /// definitions — their namespaced names, descriptions, and schemas —
    /// counted with the assembler's shaping at the last registry change.
    ///
    /// This figure is *live*, not pending: a server's tools register and
    /// unregister only when a confirm reconciles the connection set, so
    /// consumption has not changed until it has. Toggling a row moves nothing.
    pub context_tokens: usize,
    /// Change signature of the registry subset the figure was last computed
    /// from: `(definition count, Σ name + description + schema byte lengths)`.
    ///
    /// The refresh path runs every frame; this lets it skip the tokenizer
    /// entirely when nothing registered or unregistered.
    pub registry_sig: Option<(usize, usize)>,
}

impl McpPickerState {
    /// The cell slot the inspector's state lives in.
    #[must_use]
    pub fn slot() -> SlotKey {
        SlotKey::builtin("mcp", "picker")
    }
}

/// The inspector's dynamic scope.
#[must_use]
pub fn mcp_picker_scope() -> SliceScopeId {
    SliceScopeId::new("mcp", "picker")
}

/// The cell slot the inspector's state lives in.
#[must_use]
pub fn mcp_picker_slot() -> SlotKey {
    McpPickerState::slot()
}
