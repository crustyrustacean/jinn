//! The `load_attendant` tool — attach an attendant from a workflow-local
//! TOML file as a child of the calling session.
//!
//! A workflow needs attendants that belong to the *workflow*, not to the
//! user's jinn.toml: a run's attendants ship as TOML files beside the
//! workflow's prompt files, and starting the workflow is what brings them
//! alive. This tool loads one such file, builds the attendant through the
//! same path the saved-attendant picker uses, attaches it as a child of
//! the loading session, and stamps it with its source file.
//!
//! The stamp is the whole save story: a workflow-loaded attendant refuses
//! to be written into jinn.toml (see the properties save path in
//! jinn-attendant), so a workflow's tuning can never leak into the user's
//! personal configuration, and its source file stays the only writer of
//! its configuration. The stamp is per-attendant, not inherited: an
//! attendant a workflow attendant later creates is an ordinary personal
//! one.

use crate::tool_types::{ToolContext, tool_error};

use super::BoxedToolFuture;
use jinn_attendant::saved_create;
use jinn_core_types::AttendantSourcePath;
use jinn_core_types::tool_types::{ToolCall, ToolDefinition, ToolResult};
use jinn_preferences_config::schemas::AttendantEntryConfig;

/// The `load_attendant` tool definition.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "load_attendant".to_owned(),
        description: "Attach an attendant from a workflow-local TOML file as a child of \
             this session. The file uses the same shape as an [[attendant.entry]] block \
             in jinn.toml. The loaded attendant belongs to that workflow: it cannot be \
             saved to jinn.toml, and its configuration lives in that file."
            .to_owned(),
        prompt_snippet: Some(
            "Attach a workflow attendant: `load_attendant` with the attendant file's path."
                .to_owned(),
        ),
        prompt_guidelines: vec![
            "Use `load_attendant` to bring up the attendants a workflow needs, from \
             the TOML files that ship beside the workflow. The result names the created \
             attendant's session id — that is the id its messages are addressed to."
                .to_owned(),
        ],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the attendant TOML file. Relative paths resolve \
                         against this session's working directory."
                },
                "set_name": {
                    "type": "string",
                    "description": "Optional replacement for the attendant's name from the \
                         file — for labeling instances of one shared seed (e.g. \
                         \"implementor-002\"). Behavior and configuration are unaffected."
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
/// parent's *state*, not just its id), stamps the source-file marker,
/// inserts, and publishes the creation's messages through the bus in the
/// order the picker path uses.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let Some(raw_path) = string_argument(&call, "path") else {
            return tool_error(&call, "missing required argument: path");
        };
        let set_name = string_argument(&call, "set_name");
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
            // A caller that is itself an attendant cannot host one: the
            // creation path requires a non-attendant parent.
            if caller.is_attendant() {
                return tool_error(&call, "only a non-attendant session can load an attendant");
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
            Ok(text) => match parse_attendant_entry(&text) {
                Ok(entry) => entry,
                Err(reason) => return tool_error(&call, &format!("{source}: {reason}")),
            },
            Err(error) => {
                return tool_error(&call, &format!("cannot read {source}: {error}"));
            }
        };

        // Build under the read lock — `build` snapshots the parent's state —
        // then stamp, insert, and hand the caller the child's id. An
        // explicit `set_name` relabels the instance after the build: the
        // seed file's name stays the shared identity; this instance's
        // display name becomes the caller's label.
        let display_name = set_name.unwrap_or_else(|| entry.name.clone());
        let (created, messages, attendant_id) = {
            let guard = state.read();
            let Some(parent) = guard.session.get(&parent_id) else {
                return tool_error(&call, "calling session is not live");
            };
            let mut attendant = saved_create::build(&entry, parent);
            attendant.set_title(display_name.clone());
            attendant.set_workflow_source(AttendantSourcePath::new(source.clone()));
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
                "Attached attendant \"{}\" as {attendant_id} (loaded from {source}).",
                display_name
            ),
        )
    })
}

/// Why an attendant file could not be loaded as exactly one named entry.
///
/// Variants carry everything the model needs in their message: the tool
/// boundary renders a failure as tool-result prose, and a `Report`'s
/// `Display` prints only the outermost context.
#[derive(Debug, wherror::Error)]
pub enum AttendantParseError {
    /// The parsed entry carried no name.
    #[error("attendant entry has no name")]
    NoName,
    /// The file names no entry at all.
    #[error("the file names no attendant entry")]
    EmptyFile,
    /// The file carries several entries where exactly one is required.
    #[error("the file carries {count} entries; load them one file per attendant")]
    MultipleEntries {
        /// How many entries the file carried.
        count: usize,
    },
    /// The file matched none of the three accepted document shapes.
    #[error("not a valid attendant entry: {detail}")]
    NotAnEntry {
        /// The parser's complaint.
        detail: String,
    },
}

/// Parses an attendant file in any of the three shapes such a file might
/// reasonably carry: the full document shape jinn.toml uses
/// (`[[attendant.entry]]`, which nests the array under an `attendant`
/// table — the shape the shipped autocoder attendant files and the tool's
/// own description promise), the flat single-entry document
/// (`{ entry = [ ... ] }`), or one bare entry's keys. All converge on
/// exactly one entry; anything else is refused with the reason.
///
/// # Errors
///
/// Returns [`AttendantParseError`] naming why the file is not exactly one
/// named entry.
pub(crate) fn parse_attendant_entry(
    text: &str,
) -> Result<AttendantEntryConfig, error_stack::Report<AttendantParseError>> {
    if let Ok(entry) = toml::from_str::<AttendantEntryConfig>(text) {
        if entry.name.trim().is_empty() {
            return Err(AttendantParseError::NoName.into());
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
        return one_entry(&doc.attendant.entry);
    }
    #[derive(serde::Deserialize)]
    struct Wrapped {
        entry: Vec<AttendantEntryConfig>,
    }
    match toml::from_str::<Wrapped>(text) {
        Ok(wrapped) => one_entry(&wrapped.entry),
        Err(error) => Err(AttendantParseError::NotAnEntry {
            detail: error.to_string(),
        }
        .into()),
    }
}

/// Requires exactly one named entry from a parsed entry list.
fn one_entry(
    entries: &[AttendantEntryConfig],
) -> Result<AttendantEntryConfig, error_stack::Report<AttendantParseError>> {
    match entries {
        [one] => {
            if one.name.trim().is_empty() {
                return Err(AttendantParseError::NoName.into());
            }
            Ok(one.clone())
        }
        [] => Err(AttendantParseError::EmptyFile.into()),
        _ => Err(AttendantParseError::MultipleEntries {
            count: entries.len(),
        }
        .into()),
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
