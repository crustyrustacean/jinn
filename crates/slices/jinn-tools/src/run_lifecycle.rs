//! The `run_lifecycle` tool — run a session lifecycle script by name or path.
//!
//! Two script sources exist and are never combined. The user's `jinn.toml`
//! holds their own `[[session_lifecycle.script]]` entries, resolved by name;
//! a workflow ships its own `lifecycle.toml` beside its prompts, and the
//! tool reads that exact file when the call carries a `path`, with the name
//! selecting the entry inside it — one multi-entry workflow file works as
//! shipped. The sources never merge: a `path` bypasses the user's config
//! entirely, and a call supplying neither source is refused.
//!
//! jinn.toml scripts are creation-coupled: the picker runs setup only for a
//! brand-new session, and teardown replays the args stored at creation. A
//! workflow needs neither constraint — its orchestrator forks worktrees for
//! *existing* stage sessions and merges them down when a stage finishes — so
//! this tool runs a named script's setup or teardown for a session the agent
//! names, with the script's arguments supplied in the call.
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

/// The `run_lifecycle` tool definition.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "run_lifecycle".to_owned(),
        description: "Run a session lifecycle script's setup or teardown for a session, \
             with the script's arguments supplied here. The script comes from jinn.toml \
             by name, or — when a path is given — from that exact file, with the name \
             selecting the entry inside it. Setup adopts the last path the script \
             prints as the session's working directory. Values are shell-quoted \
             before substitution, so they cannot alter the command."
            .to_owned(),
        prompt_snippet: Some(
            "Run a lifecycle script: `run_lifecycle` with the script name (and its \
             file's path for a workflow script), the action, and its arguments."
                .to_owned(),
        ),
        prompt_guidelines: vec![
            "Use `run_lifecycle` to give a session its scripted environment — a \
             worktree, a checkout — or to tear that environment down. Supply every \
             argument the script's placeholders name; values are quoted for you. \
             A workflow's scripts live in its `lifecycle.toml`: pass that path and \
             the script's name."
                .to_owned(),
        ],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "script": {
                    "type": "string",
                    "description": "The lifecycle script's name — from jinn.toml, or \
                         from the file the path argument names."
                },
                "path": {
                    "type": "string",
                    "description": "Path to a lifecycle TOML file to read the script \
                         from instead of jinn.toml (e.g. a workflow's lifecycle.toml)."
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

/// Executes `run_lifecycle`.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        // The optional path must be read first: a blank script with a path
        // present is a different refusal than a blank script alone.
        let raw_path = string_argument(&call, "path").filter(|p| !p.trim().is_empty());
        let raw_script = string_argument(&call, "script");
        let script_name = match (&raw_script, &raw_path) {
            (Some(name), _) if !name.trim().is_empty() => name.clone(),
            (Some(_), Some(_)) => {
                return tool_error(
                    &call,
                    "script is required with path: the name selects the entry inside \
                     the file",
                );
            }
            // A present-but-blank name without a path is not a schema miss —
            // JSON `""` satisfies `required` — so it keeps the historic
            // refusal: the config resolution reports the unknown script ""
            // with the known list.
            (Some(name), None) => name.clone(),
            (None, _) => return tool_error(&call, "missing required argument: script"),
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
            return tool_error(&call, "run_lifecycle is unavailable without shared state");
        };
        let Some(bus) = ctx.bus.clone() else {
            return tool_error(&call, "run_lifecycle is unavailable without a bus");
        };
        let Some(system) = ctx.trouper_system.clone() else {
            return tool_error(
                &call,
                "run_lifecycle is unavailable without the actor system",
            );
        };

        // Resolve the script and render the command before touching state:
        // an unreadable file, an unknown script, or a missing argument must
        // fail without any side effect. The two sources never combine: a
        // path reads that file and never consults the user's config. The
        // source is recorded with the session so its teardown re-reads the
        // same place after a restart.
        let (raw_command, source_path) = match raw_path {
            Some(path) => match resolve_from_path(&path, &ctx, &script_name, &action, kind) {
                Ok(resolved) => resolved,
                Err(reason) => return tool_error(&call, &reason),
            },
            None => match resolve_from_config(&ctx, &script_name, &action, kind) {
                Ok(resolved) => resolved,
                Err(reason) => return tool_error(&call, &reason),
            },
        };
        let command = match render_command(&raw_command, &args) {
            Ok(rendered) => rendered,
            Err(reason) => return tool_error(&call, &reason),
        };

        // Teardown replays the session's *stored* args (the actor re-renders
        // from the recorded source and ignores the command we publish), so
        // the tool's coordinates must land on the session before the command
        // does. State-before-publish ordering is load-bearing.
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
            session.set_lifecycle_source(source_path);
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

/// Resolves the script's raw command from the user's jinn.toml by name.
///
/// The only reader of the config source: a call carrying a `path` never
/// reaches this function. The source is `None` — jinn.toml is the ambient
/// source, not one a session records.
fn resolve_from_config(
    ctx: &ToolContext,
    script_name: &str,
    action: &str,
    kind: AwaitKind,
) -> Result<(String, Option<String>), String> {
    let scripts = ctx
        .config
        .get_list::<SessionLifecycle>()
        .map_err(|error| format!("cannot read lifecycle scripts: {error}"))?;
    let Some(script) = scripts.iter().find(|s| s.name == script_name) else {
        return Err(format!(
            "unknown lifecycle script \"{script_name}\"; known: {}",
            scripts
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };
    half_of(script, script_name, action, kind).map(|raw| (raw.to_owned(), None))
}

/// Resolves the script's raw command from the file at `path`, resolved
/// against the calling session's cwd the way every file-facing tool
/// resolves a path argument. The resolved path is returned as the source —
/// the session records it so teardown re-reads the same file.
fn resolve_from_path(
    path: &str,
    ctx: &ToolContext,
    script_name: &str,
    action: &str,
    kind: AwaitKind,
) -> Result<(String, Option<String>), String> {
    let resolved = crate::tool_paths::resolve_path(path, &ctx.cwd);
    let source = resolved.display().to_string();
    let text = std::fs::read_to_string(&resolved)
        .map_err(|error| format!("cannot read {source}: {error}"))?;
    let scripts =
        jinn_preferences_config::schemas::session_lifecycle::parse_lifecycle_document(&text)
            .map_err(|error| format!("{source}: {error}"))?;
    let Some(script) = scripts.iter().find(|s| s.name == script_name) else {
        return Err(format!(
            "{source}: no lifecycle script named \"{script_name}\"; the file defines: {}",
            scripts
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };
    half_of(script, script_name, action, kind).map(|raw| (raw.to_owned(), Some(source)))
}

/// Takes the requested half of a script as the raw template string,
/// refusing a builtin command — the tool runs shell, not builtins.
fn half_of<'a>(
    script: &'a SessionLifecycle,
    script_name: &str,
    action: &str,
    kind: AwaitKind,
) -> Result<&'a str, String> {
    let half = match kind {
        AwaitKind::Setup => script.setup.as_ref(),
        AwaitKind::Teardown => script.teardown.as_ref(),
    };
    match half {
        Some(LifecycleCommand::Shell(cmd)) => Ok(cmd.as_str()),
        Some(LifecycleCommand::Builtin(_)) => Err(format!(
            "lifecycle script \"{script_name}\" uses a builtin {action}, \
             which this tool does not run"
        )),
        None => Err(format!(
            "lifecycle script \"{script_name}\" defines no {action} command"
        )),
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
