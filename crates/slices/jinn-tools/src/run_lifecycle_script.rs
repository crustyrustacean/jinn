//! The `run_lifecycle_script` tool — run a named session lifecycle script.
//!
//! jinn.toml's `[[session_lifecycle.script]]` entries are creation-coupled
//! today: the picker runs setup only for a brand-new session, and teardown
//! replays the args stored at creation. A workflow needs neither constraint
//! — its orchestrator forks worktrees for *existing* stage sessions and
//! merges them down when a stage finishes — so this tool runs a named
//! script's setup or teardown for a session the agent names, with the
//! script's arguments supplied in the call.
//!
//! Everything dangerous is borrowed, not reimplemented: argument values
//! render through `CommandTemplate`'s shell-quoted path (the same one the
//! arg-input popup feeds), execution goes through the lifecycle actor's
//! terminal-isolated runner, and a setup that prints a directory still
//! moves the session there. The tool contributes exactly one new thing —
//! named arguments reaching an existing session — and composes the rest.

use std::collections::BTreeMap;

use unicode_segmentation::UnicodeSegmentation;

use crate::lifecycle_completion_listener_actor::{
    AwaitKind, LifecycleCompletion, LifecycleCompletionListenerActor,
    LifecycleCompletionListenerDeps,
};
use crate::tool_types::{ToolContext, tool_error};

use super::BoxedToolFuture;
use jinn_core_types::session_id::SessionId;
use jinn_core_types::tool_types::{ToolCall, ToolDefinition, ToolResult};
use jinn_preferences_config::schemas::{LifecycleCommand, SessionLifecycle};
use jinn_session_lifecycle_msg::command_template::CommandTemplate;
use jinn_session_lifecycle_msg::{RunSessionSetup, RunSessionTeardown};

/// The `run_lifecycle_script` tool definition.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "run_lifecycle_script".to_owned(),
        description: "Run a named session lifecycle script's setup or teardown for a \
             session, with the script's arguments supplied here. Setup adopts the last \
             path the script prints as the session's working directory. Values are \
             shell-quoted before substitution, so they cannot alter the command."
            .to_owned(),
        prompt_snippet: Some(
            "Run a lifecycle script: `run_lifecycle_script` with the script name, the \
             action, and its arguments."
                .to_owned(),
        ),
        prompt_guidelines: vec![
            "Use `run_lifecycle_script` to give a session its scripted environment — \
             a worktree, a checkout — or to tear that environment down. Supply every \
             argument the script's placeholders name; values are quoted for you."
                .to_owned(),
        ],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "script": {
                    "type": "string",
                    "description": "The lifecycle script's name from jinn.toml."
                },
                "action": {
                    "type": "string",
                    "enum": ["setup", "teardown"],
                    "description": "Which half of the script to run."
                },
                "session_id": {
                    "type": "string",
                    "description": "The session the script runs for."
                },
                "args": {
                    "type": "object",
                    "description": "Named arguments for the script's <placeholder> \
                         parameters, keyed by placeholder name.",
                    "additionalProperties": { "type": "string" }
                }
            },
            "required": ["script", "action", "session_id", "args"]
        }),
        server_tool_type: None,
    }
}

/// Executes `run_lifecycle_script`.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let Some(script_name) = string_argument(&call, "script") else {
            return tool_error(&call, "missing required argument: script");
        };
        let Some(action) = string_argument(&call, "action") else {
            return tool_error(&call, "missing required argument: action");
        };
        let kind = match action.as_str() {
            "setup" => AwaitKind::Setup,
            "teardown" => AwaitKind::Teardown,
            other => {
                return tool_error(
                    &call,
                    &format!("unknown action \"{other}\"; expected setup or teardown"),
                );
            }
        };
        let Some(raw_id) = string_argument(&call, "session_id") else {
            return tool_error(&call, "missing required argument: session_id");
        };
        let Some(target_id) = SessionId::try_from_string(&raw_id) else {
            return tool_error(&call, &format!("unknown session: {raw_id}"));
        };
        let args = match map_argument(&call, "args") {
            Ok(args) => args,
            Err(reason) => return tool_error(&call, &reason),
        };
        let Some(state) = ctx.state.clone() else {
            return tool_error(
                &call,
                "run_lifecycle_script is unavailable without shared state",
            );
        };
        let Some(bus) = ctx.bus.clone() else {
            return tool_error(&call, "run_lifecycle_script is unavailable without a bus");
        };
        let Some(system) = ctx.trouper_system.clone() else {
            return tool_error(
                &call,
                "run_lifecycle_script is unavailable without the actor system",
            );
        };

        // Resolve the script and render the command before touching state:
        // an unknown script or a missing argument must fail without any
        // side effect.
        let (command, raw_command) = match ctx.config.get_list::<SessionLifecycle>() {
            Ok(scripts) => {
                let Some(script) = scripts.iter().find(|s| s.name == script_name) else {
                    return tool_error(
                        &call,
                        &format!(
                            "unknown lifecycle script \"{script_name}\"; known: {}",
                            scripts
                                .iter()
                                .map(|s| s.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    );
                };
                let Some(raw) = half_of(script, kind) else {
                    return tool_error(
                        &call,
                        &format!("lifecycle script \"{script_name}\" defines no {action} command"),
                    );
                };
                let raw_command = raw.to_owned();
                match LifecycleCommand::Shell(raw_command.clone()) {
                    LifecycleCommand::Shell(cmd) => match render_command(&cmd, &args) {
                        Ok(rendered) => (rendered, raw_command),
                        Err(reason) => return tool_error(&call, &reason),
                    },
                    LifecycleCommand::Builtin(_) => {
                        return tool_error(
                            &call,
                            &format!(
                                "lifecycle script \"{script_name}\" uses a builtin \
                                 {action}, which this tool does not run"
                            ),
                        );
                    }
                }
            }
            Err(error) => {
                return tool_error(&call, &format!("cannot read lifecycle scripts: {error}"));
            }
        };

        // Teardown replays the session's *stored* args (the actor re-renders
        // from config and ignores the command we publish), so the tool's args
        // must land on the session before the command does.
        if kind == AwaitKind::Teardown {
            let mut guard = state.write();
            let Some(session) = guard.session.get_mut(&target_id) else {
                return tool_error(&call, &format!("unknown session: {target_id}"));
            };
            // The actor re-renders teardown from the RAW template plus
            // these stored args, so the args are ordered against the raw
            // template's parameter order — not the rendered command, which
            // has no placeholders left.
            let ordered = order_args(&CommandTemplate::parse(&raw_command), &args);
            session.set_lifecycle_name(Some(script_name.clone()));
            session.set_lifecycle_args(ordered);
        }

        // Listener before publication: no completion event can slip past
        // the subscription.
        let (tx, rx) = tokio::sync::oneshot::channel::<LifecycleCompletion>();
        LifecycleCompletionListenerActor::spawn(LifecycleCompletionListenerDeps {
            system: system.clone(),
            bus: bus.clone(),
            session_id: target_id.clone(),
            kind,
            completion: tx,
        })
        .await;

        match kind {
            AwaitKind::Setup => {
                // The actor honors the command we publish: it spawns the
                // rendered command terminal-isolated in the session's cwd
                // and adopts the last stdout line as the session's cwd.
                // Args ride in raw-template parameter order, matching what
                // a teardown replay would read back.
                let template = CommandTemplate::parse(&raw_command);
                bus.publish(RunSessionSetup {
                    session_id: target_id.clone(),
                    command: command.clone(),
                    args: order_args(&template, &args),
                    lifecycle_command: Some(LifecycleCommand::Shell(command.clone())),
                })
                .await;
            }
            AwaitKind::Teardown => {
                bus.publish(RunSessionTeardown {
                    session_id: target_id.clone(),
                    command: command.clone(),
                    args: Vec::new(),
                })
                .await;
            }
        }

        // Bound the wait: a script that hangs cannot hang the tool batch
        // forever. The script keeps running past the timeout either way —
        // this reports, it does not kill.
        let completion = tokio::time::timeout(std::time::Duration::from_secs(300), rx).await;
        match completion {
            Ok(Ok(done)) => match done.error {
                None => {
                    let where_to = done
                        .cwd
                        .map(|cwd| format!(" Session cwd: {}.", cwd.display()))
                        .unwrap_or_default();
                    succeeded(
                        call,
                        &format!("{action} for \"{script_name}\" completed.{where_to}"),
                    )
                }
                Some(error) => failed(
                    call,
                    &format!("{action} for \"{script_name}\" failed: {error}"),
                ),
            },
            _ => failed(
                call,
                &format!(
                    "{action} for \"{script_name}\" did not report completion within \
                     the wait budget; it may still be running"
                ),
            ),
        }
    })
}

/// The script's command for one half, as the raw template string.
fn half_of<'a>(script: &'a SessionLifecycle, kind: AwaitKind) -> Option<&'a str> {
    match kind {
        AwaitKind::Setup => script.setup.as_ref().and_then(|c| match c {
            LifecycleCommand::Shell(cmd) => Some(cmd.as_str()),
            LifecycleCommand::Builtin(_) => None,
        }),
        AwaitKind::Teardown => script.teardown.as_ref().and_then(|c| match c {
            LifecycleCommand::Shell(cmd) => Some(cmd.as_str()),
            LifecycleCommand::Builtin(_) => None,
        }),
    }
}

/// Renders the command through a single scan-and-emit pass that shell-quotes
/// every substituted value.
///
/// `CommandTemplate::render` quotes each value — but it substitutes with
/// sequential `String::replace` passes, so a *value* carrying a later
/// placeholder's shape (`<base> ; rm -rf /`) would be found by the next
/// pass and substituted again, breaking out of its quotes. This renderer
/// keeps the template's quoting (`shell_quote`) but emits left-to-right in
/// one pass: emitted output is never re-scanned, so a value can never be
/// mistaken for a placeholder.
pub(crate) fn render_command(raw: &str, args: &BTreeMap<String, String>) -> Result<String, String> {
    use jinn_session_lifecycle_msg::command_template::Param;
    use jinn_session_lifecycle_msg::command_template::shell_quote;
    let template = CommandTemplate::parse(raw);
    for param in template.params() {
        match param {
            Param::Named(name) => {
                if !args.contains_key(name) {
                    return Err(format!(
                        "missing argument for placeholder <{name}>; the script names: {}",
                        template
                            .params()
                            .iter()
                            .filter_map(|p| match p {
                                Param::Named(n) => Some(n.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            Param::Positional(n) => {
                return Err(format!(
                    "script uses positional placeholder ${n}; this tool supplies named \
                     arguments keyed by placeholder name"
                ));
            }
            Param::Splat => {
                return Err(
                    "script uses a splat placeholder; this tool supplies named arguments"
                        .to_owned(),
                );
            }
        }
    }
    // Scan the source grapheme-wise, the way parse does: every `<name>`
    // span in the original text becomes the quoted value; every other
    // grapheme copies through. Emitted values are never revisited.
    let mut out = String::with_capacity(raw.len());
    let graphemes: Vec<&str> = raw.graphemes(true).collect();
    let mut i = 0;
    while i < graphemes.len() {
        if graphemes[i] == "<"
            && let Some((Param::Named(name), width)) = scan_named(&graphemes, i)
        {
            // Missing names were refused above.
            let value = args.get(&name).map(String::as_str).unwrap_or_default();
            out.push_str(&shell_quote(value));
            i += width;
            continue;
        }
        out.push_str(graphemes[i]);
        i += 1;
    }
    Ok(out)
}

/// Scans one `<name>` span starting at `start`; returns the name and the
/// span's grapheme width. Mirrors the lifecycle crate's own scan.
fn scan_named(
    graphemes: &[&str],
    start: usize,
) -> Option<(jinn_session_lifecycle_msg::command_template::Param, usize)> {
    use jinn_session_lifecycle_msg::command_template::Param;
    let mut end = start + 1;
    while end < graphemes.len() && graphemes[end] != "<" {
        if graphemes[end] == ">" {
            let name: String = graphemes[start + 1..end].concat();
            if name.is_empty() {
                return None;
            }
            return Some((Param::Named(name), end - start + 1));
        }
        end += 1;
    }
    None
}

/// Orders the named argument map positionally the way the session's stored
/// `lifecycle_args` expect (template parameter order).
pub(crate) fn order_args(
    template: &CommandTemplate,
    args: &BTreeMap<String, String>,
) -> Vec<String> {
    template
        .params()
        .iter()
        .filter_map(|p| match p {
            jinn_session_lifecycle_msg::command_template::Param::Named(name) => {
                args.get(name).cloned()
            }
            _ => None,
        })
        .collect()
}

/// Reads one string argument from the call's JSON arguments.
fn string_argument(call: &ToolCall, key: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(&call.arguments)
        .ok()?
        .get(key)?
        .as_str()
        .map(str::to_owned)
}

/// Reads one string-map argument from the call's JSON arguments.
fn map_argument(call: &ToolCall, key: &str) -> Result<BTreeMap<String, String>, String> {
    let value: serde_json::Value = serde_json::from_str(&call.arguments)
        .map_err(|error| format!("malformed tool arguments: {error}"))?;
    match value.get(key) {
        Some(serde_json::Value::Object(map)) => {
            let mut args = BTreeMap::new();
            for (k, v) in map {
                match v.as_str() {
                    Some(s) => {
                        args.insert(k.clone(), s.to_owned());
                    }
                    None => return Err(format!("argument \"{k}\" must be a string")),
                }
            }
            Ok(args)
        }
        _ => Err(format!("missing required argument: {key}")),
    }
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

/// Builds a failed result for a script that ran and reported failure.
fn failed(call: ToolCall, content: &str) -> ToolResult {
    ToolResult {
        tool_call_id: call.id,
        name: call.name,
        content: content.to_owned(),
        success: false,
        full_content: None,
        truncation: None,
        pin_position: None,
    }
}
