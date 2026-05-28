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

//! Task list tool registry.
//!
//! Wires each task list tool module into a list of (definition, execute) pairs
//! for registration by the tool orchestrator.

use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition};
use crate::feat::tools_actor::BoxedToolFuture;

use super::{add_phase, add_task, complete_task, get_phase, get_task_list};

/// A built-in tool entry: its definition paired with its execute function.
pub type BuiltinToolEntry = (ToolDefinition, fn(ToolCall, ToolContext) -> BoxedToolFuture);

/// Returns all task list tool entries (definition + execute function).
///
/// Used by the tool orchestrator to register task list tools at activation.
pub fn tool_entries() -> Vec<BuiltinToolEntry> {
    vec![
        (
            add_phase::definition(),
            add_phase::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            add_task::definition(),
            add_task::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            complete_task::definition(),
            complete_task::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            get_task_list::definition(),
            get_task_list::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
        (
            get_phase::definition(),
            get_phase::execute as fn(ToolCall, ToolContext) -> BoxedToolFuture,
        ),
    ]
}

/// Returns all task list tool definitions (for prompt injection).
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        add_phase::definition(),
        add_task::definition(),
        complete_task::definition(),
        get_task_list::definition(),
        get_phase::definition(),
    ]
}
