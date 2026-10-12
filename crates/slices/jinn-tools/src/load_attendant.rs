//! The `load_attendant` tool — attach a cast member from a workflow-local
//! TOML file as a child of the calling session.
//!
//! A workflow needs attendants that belong to the *workflow*, not to the
//! user's jinn.toml: a run's stage cast ships beside the workflow's
//! prompt files, and starting the workflow is what brings them alive. This
//! tool loads one cast file, builds the attendant through the same path
//! the saved-attendant picker uses, attaches it as a child of the loading
//! session, and stamps it as a workflow-cast member naming its source.
//!
//! The stamp is the whole save story: a cast member refuses to be written
//! into jinn.toml (see the properties save path in jinn-attendant), so a
//! workflow's tuning can never leak into the user's personal configuration,
//! and its source file stays the only writer of its configuration. The
//! stamp is per-attendant, not inherited: an attendant a cast member later
//! creates is an ordinary personal one.

use crate::tool_types::{ToolContext, tool_error};

use super::BoxedToolFuture;
use jinn_attendant::saved_create;
use jinn_core_types::tool_types::{ToolCall, ToolDefinition, ToolResult};
use jinn_preferences_config::schemas::AttendantEntryConfig;

/// The `load_attendant` tool definition.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "load_attendant".to_owned(),
        description: "Attach an attendant from a workflow-local TOML file as a child of \
             this session. The file uses the same shape as an [[attendant.entry]] block \
             in jinn.toml. The loaded attendant is a workflow cast member: it cannot be \
             saved to jinn.toml, and its configuration lives in that file."
            .to_owned(),
        prompt_snippet: Some(
            "Attach a workflow cast member: `load_attendant` with the cast file's path.".to_owned(),
        ),
        prompt_guidelines: vec![
            "Use `load_attendant` to bring up the attendants a workflow needs, from \
             cast files that ship beside the workflow. The result names the created \
             attendant's session id — that is the id its messages are addressed to."
                .to_owned(),
        ],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the cast TOML file. Relative paths resolve \
                         against this session's working directory."
                }
            },
            "required": ["path"]
        }),
        server_tool_type: None,
    }
}

/// Executes `load_attendant`.
///
/// Parses the file as a single `[[attendant.entry]]`-shaped document —
/// either a bare entry's keys or a document wrapping one `entry` array —
/// then builds the attendant under the state read lock (the build needs the
/// parent's *state*, not just its id), stamps the cast marker, inserts, and
/// publishes the creation's messages through the bus in the order the
/// picker path uses.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let Some(raw_path) = string_argument(&call, "path") else {
            return tool_error(&call, "missing required argument: path");
        };
        let Some(state) = ctx.state.clone() else {
            return tool_error(&call, "load_attendant is unavailable without shared state");
        };
        let Some(bus) = ctx.bus.clone() else {
            return tool_error(&call, "load_attendant is unavailable without a bus");
        };
        let Some(parent_id) = ctx.session_id.clone() else {
            return tool_error(
                &call,
                "load_attendant is unavailable without a session context",
            );
        };

        // Resolve the path against the session's cwd, the same relative-base
        // every file-facing tool honors.
        let cwd = {
            let guard = state.read();
            let Some(caller) = guard.session.get(&parent_id) else {
                return tool_error(&call, "calling session is not live");
            };
            // A caller that is itself an attendant cannot host a cast: the
            // creation path requires a non-attendant parent.
            if caller.is_attendant() {
                return tool_error(&call, "only a non-attendant session can load a cast member");
            }
            caller.cwd().to_path_buf()
        };
        let path = {
            let candidate = std::path::Path::new(&raw_path);
            if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                cwd.join(candidate)
            }
        };
        let source = path.display().to_string();
        let entry = match std::fs::read_to_string(&path) {
            Ok(text) => match parse_cast_entry(&text) {
                Ok(entry) => entry,
                Err(reason) => return tool_error(&call, &format!("{source}: {reason}")),
            },
            Err(error) => {
                return tool_error(&call, &format!("cannot read {source}: {error}"));
            }
        };

        // Build under the read lock — `build` snapshots the parent's state —
        // then stamp, insert, and hand the caller the child's id.
        let (created, messages, attendant_id) = {
            let guard = state.read();
            let Some(parent) = guard.session.get(&parent_id) else {
                return tool_error(&call, "calling session is not live");
            };
            let mut attendant = saved_create::build(&entry, parent);
            attendant.set_workflow_cast(source.clone());
            let messages = saved_create::creation_messages(&parent_id, &attendant);
            let id = attendant.session_id().clone();
            (attendant, messages, id)
        };
        {
            let mut guard = state.write();
            guard.session.insert(created);
            if let Some(parent) = guard.session.get_mut(&parent_id) {
                parent.mark_interacted();
            }
        }
        for closure in messages.messages {
            closure(&bus);
        }

        succeeded(
            call,
            &format!(
                "Attached attendant \"{}\" as {attendant_id} (workflow cast member from {source}).",
                entry.name
            ),
        )
    })
}

/// Parses a cast file in any of the three shapes a cast file might
/// reasonably carry: the full document shape jinn.toml uses
/// (`[[attendant.entry]]`, which nests the array under an `attendant`
/// table — the shape the shipped autocoder cast files and the tool's own
/// description promise), the flat single-member document (`{ entry =
/// [ ... ] }`), or one bare entry's keys. All converge on exactly one
/// member; anything else is refused with the reason.
pub(crate) fn parse_cast_entry(text: &str) -> Result<AttendantEntryConfig, String> {
    if let Ok(entry) = toml::from_str::<AttendantEntryConfig>(text) {
        if entry.name.trim().is_empty() {
            return Err("cast entry has no name".to_owned());
        }
        return Ok(entry);
    }
    #[derive(serde::Deserialize)]
    struct Document {
        attendant: Section,
    }
    #[derive(serde::Deserialize)]
    struct Section {
        entry: Vec<AttendantEntryConfig>,
    }
    if let Ok(doc) = toml::from_str::<Document>(text) {
        return one_member(&doc.attendant.entry);
    }
    #[derive(serde::Deserialize)]
    struct Wrapped {
        entry: Vec<AttendantEntryConfig>,
    }
    match toml::from_str::<Wrapped>(text) {
        Ok(wrapped) => one_member(&wrapped.entry),
        Err(error) => Err(format!("not a valid cast entry: {error}")),
    }
}

/// Requires exactly one named member from a parsed entry list.
fn one_member(entries: &[AttendantEntryConfig]) -> Result<AttendantEntryConfig, String> {
    match entries {
        [one] => {
            if one.name.trim().is_empty() {
                return Err("cast entry has no name".to_owned());
            }
            Ok(one.clone())
        }
        [] => Err("cast file names no entry".to_owned()),
        _ => Err(format!(
            "cast file carries {} entries; load them one file per member",
            entries.len()
        )),
    }
}

/// Reads one string argument from the call's JSON arguments.
fn string_argument(call: &ToolCall, key: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(&call.arguments)
        .ok()?
        .get(key)?
        .as_str()
        .map(str::to_owned)
}

/// Builds a successful result.
fn succeeded(call: ToolCall, content: &str) -> ToolResult {
    ToolResult {
        tool_call_id: call.id,
        name: call.name,
        content: content.to_owned(),
        success: true,
        full_content: None,
        truncation: None,
        pin_position: None,
    }
}
