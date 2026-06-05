//! Hashline edit built-in tool — performs LINE#HASH-anchored file edits.
//!
//! Replaces the old exact-text `str_replace` edit tool. The LLM reads file
//! content annotated with `LINE#HASH:` tags and targets edits to those anchors
//! instead of reproducing old text verbatim. After a successful edit, fresh
//! anchors are returned for the changed region so the LLM can chain edits
//! without re-reading.

mod engine;
pub(crate) mod hash;
mod line_ending;
mod response;

use std::path::{Path, PathBuf};

use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};

use super::BoxedToolFuture;
use engine::{LinesInput, RawEdit, apply_hashline_edits, resolve_edit_anchors, validate_anchors};
use line_ending::{detect_line_ending, normalize_to_lf, restore_line_endings, strip_bom};
use response::{build_anchor_block, format_noop_response, format_success_response};

/// Construct a failure [`ToolResult`] from a `call` and error `content`.
fn err_result(call: &ToolCall, content: String) -> ToolResult {
    ToolResult {
        tool_call_id: call.id.clone(),
        name: call.name.clone(),
        content,
        success: false,
        full_content: None,
        truncation: None,
    }
}

/// Construct a success [`ToolResult`] from a `call` and `content`.
fn ok_result(call: &ToolCall, content: String) -> ToolResult {
    ToolResult {
        tool_call_id: call.id.clone(),
        name: call.name.clone(),
        content,
        success: true,
        full_content: None,
        truncation: None,
    }
}

/// Returns the tool definition for the `edit` built-in tool.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "edit".to_owned(),
        description: r#"Patch a UTF-8 text file using anchors from `read`.

`read` output looks like this:
  1#HH|first line
  2#VR|fn main() {
  3#SK|}

`1#HH|` is a line anchor. The `#` separates line number from hash; the `|` separates hash from content.
Copy the `LINE#HASH` part (e.g. `2#VR`) into `pos` / `end`.

Examples — always wrap operations in the `edits` array:
  Replace line 2:
    {"path":"f.rs","edits":[{"op":"replace","pos":"2#VR","lines":["fn main() {"]]}
  Replace lines 2–3:
    {"path":"f.rs","edits":[{"op":"replace","pos":"2#VR","end":"3#SK","lines":["fn main() {","}"]}]}
  Insert after line 2:
    {"path":"f.rs","edits":[{"op":"append","pos":"2#VR","lines":["    println!(\"hi\");"]}]}
  Insert before line 2:
    {"path":"f.rs","edits":[{"op":"prepend","pos":"2#VR","lines":["// header"]}]}
  Append at end of file:
    {"path":"f.rs","edits":[{"op":"append","lines":["// trailing"]]}

Rules:
- One `edit` call per file. All operations go in the `edits` array.
- Anchors within one call must come from the same `read` of that file.
- `lines` is literal file content — no `LINE#HASH|` prefix, no `+`/`-` markers. Match indentation exactly.
- Do not guess or construct anchors — always copy them from `read` output.
- Do not emit overlapping edits — merge them into one.
- `path` is required on every call."#.to_owned(),
        prompt_snippet: Some("Edit a text file via LINE#HASH anchors from read".to_owned()),
        prompt_guidelines: vec![
            "Use `edit` to change files using LINE#HASH anchors from `read`.".to_owned(),
            "Batch all edits for one file in a single call — edits are applied atomically bottom-up.".to_owned(),
            "On success, use the returned `--- Anchors ---` block for nearby follow-up edits without re-reading. For distant follow-ups, call `read` again.".to_owned(),
            "On stale anchor errors, use the `>>> LINE#HASH` lines shown in the error to retry.".to_owned(),
        ],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit (relative or absolute)"
                },
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "op": {
                                "type": "string",
                                "description": "Edit operation: \"replace\", \"append\", or \"prepend\""
                            },
                            "pos": {
                                "type": "string",
                                "description": "Anchor from read output — either \"LINE#HASH\" (e.g. \"5#WS\") or the full display line (e.g. \"5#WS|content\"). Required for replace. Optional for append/prepend."
                            },
                            "end": {
                                "type": "string",
                                "description": "Anchor for inclusive end of a replace range. Same format as pos."
                            },
                            "lines": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Replacement or inserted lines. Literal file content, no prefixes."
                            },
                        },
                        "required": ["op"],
                        "additionalProperties": false
                    },
                    "description": "One or more edit operations to apply atomically."
                }
            },
            "required": ["path", "edits"],
            "additionalProperties": false
        }),
        server_tool_type: None,
    }
}

/// Resolves a path against the CWD if relative, returns absolute as-is.
fn resolve_path(path: &str, cwd: &Path) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_owned()
    } else {
        cwd.join(p)
    }
}

/// Executes the `edit` built-in tool.
/// Executes the `edit` built-in tool.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let (path, raw_edits) = match parse_args(&call.arguments) {
            Ok(v) => v,
            Err(e) => return err_result(&call, format!("failed to parse arguments: {e}")),
        };

        if path.is_empty() {
            return err_result(
                &call,
                "`path` is required. Provide the file path, e.g. \"src/main.rs\".".to_owned(),
            );
        }

        if raw_edits.is_empty() {
            return err_result(&call, "No edits provided.".to_owned());
        }

        let resolved = resolve_path(&path, &ctx.cwd);

        let raw_content = match tokio::fs::read_to_string(&resolved).await {
            Ok(c) => c,
            Err(e) => {
                return err_result(
                    &call,
                    format!("failed to read file '{}': {e}", resolved.display()),
                );
            }
        };

        // Preserve original file permissions
        let orig_mode = std::fs::metadata(&resolved).ok().map(|m| m.permissions());

        // Strip BOM and detect line endings
        let (content_without_bom, bom) = strip_bom(&raw_content);
        let detected_ending = detect_line_ending(content_without_bom);

        // Normalize to LF for processing
        let normalized = normalize_to_lf(content_without_bom);

        let edits = match resolve_edit_anchors(&raw_edits) {
            Ok(e) => e,
            Err(e) => return err_result(&call, e),
        };

        // Validate anchors against current file content
        let file_lines: Vec<&str> = normalized.split('\n').collect();
        let mismatches = validate_anchors(&edits, &file_lines);
        if !mismatches.is_empty() {
            let error_msg = engine::format_mismatch_error(&mismatches, &file_lines);
            return err_result(&call, error_msg);
        }

        let edit_result = match apply_hashline_edits(&normalized, &edits) {
            Ok(r) => r,
            Err(e) => return err_result(&call, e),
        };

        if normalized == edit_result.content {
            let response_text =
                format_noop_response(&path, &edit_result.noop_edits, &edit_result.warnings);
            return ok_result(&call, response_text);
        }

        // Restore line endings and BOM
        let restored = restore_line_endings(&edit_result.content, detected_ending);
        let final_content = match bom {
            Some(b) => format!("{b}{restored}"),
            None => restored,
        };

        if let Err(msg) = write_atomic(&resolved, &final_content, orig_mode.as_ref()).await {
            return err_result(&call, msg);
        }

        // Build anchor block for chaining
        let anchor_block = build_anchor_block(
            &edit_result.content,
            edit_result.first_changed_line,
            edit_result.last_changed_line,
        );
        let response_text = format_success_response(&path, &anchor_block, &edit_result.warnings);

        ok_result(&call, response_text)
    })
}

/// Writes `content` to `resolved` atomically via temp-file + rename, preserving permissions.
async fn write_atomic(
    resolved: &Path,
    content: &str,
    orig_mode: Option<&std::fs::Permissions>,
) -> Result<(), String> {
    let tmp_path = resolved.with_extension("hashline-tmp");
    if let Err(e) = tokio::fs::write(&tmp_path, content).await {
        return Err(format!(
            "failed to write temp file '{}': {e}",
            tmp_path.display()
        ));
    }

    if let Some(mode) = orig_mode
        && let Err(e) = std::fs::set_permissions(&tmp_path, mode.clone())
    {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!("failed to set permissions on temp file: {e}"));
    }

    if let Err(e) = std::fs::rename(&tmp_path, resolved) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!(
            "failed to rename temp file to '{}': {e}",
            resolved.display()
        ));
    }
    Ok(())
}

/// Parses the arguments from the tool call JSON.
fn parse_args(raw: &str) -> Result<(String, Vec<RawEdit>), serde_json::Error> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    let path = v
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();

    let edits = v
        .get("edits")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let op = item.get("op")?.as_str()?.to_owned();
                    let pos = item.get("pos").and_then(|v| v.as_str()).map(String::from);
                    let end = item.get("end").and_then(|v| v.as_str()).map(String::from);
                    let lines = item.get("lines").map(|v| {
                        if v.is_null() {
                            LinesInput::Null
                        } else {
                            LinesInput::Array(
                                v.as_array()
                                    .map(|a| {
                                        a.iter()
                                            .filter_map(|s| s.as_str().map(String::from))
                                            .collect()
                                    })
                                    .unwrap_or_default(),
                            )
                        }
                    });
                    let old_text = item
                        .get("oldText")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let new_text = item
                        .get("newText")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    Some(RawEdit {
                        op,
                        pos,
                        end,
                        lines,
                        old_text,
                        new_text,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok((path, edits))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext};
    use std::path::PathBuf;

    fn test_ctx() -> ToolContext {
        ToolContext {
            cwd: PathBuf::from("/tmp"),
            timeout: None,
            bash_default_timeout: None,
            state: None,
            session_id: None,
            app_paths: crate::common::app_paths::AppPaths::default(),
            sink: None,
            shell: "/bin/sh".to_owned(),
            max_output_lines: None,
            max_output_bytes: None,
        }
    }

    fn test_ctx_with_cwd(cwd: PathBuf) -> ToolContext {
        ToolContext {
            cwd,
            timeout: None,
            bash_default_timeout: None,
            state: None,
            session_id: None,
            app_paths: crate::common::app_paths::AppPaths::default(),
            sink: None,
            shell: "/bin/sh".to_owned(),
            max_output_lines: None,
            max_output_bytes: None,
        }
    }

    #[rstest::rstest]
    fn definition_has_correct_name() {
        // Given the edit tool definition.
        let def = definition();

        // Then it has the name "edit".
        assert_eq!(def.name, "edit");
        assert!(def.prompt_snippet.is_some());
        assert!(!def.prompt_guidelines.is_empty());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_error_on_empty_edits() {
        // Given a temp file with content.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").expect("write temp file");

        let call = ToolCall {
            id: "call_1".to_owned(),
            name: "edit".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "edits": []
            })
            .to_string(),
        };

        // When executing with empty edits.
        let result = execute(call, test_ctx()).await;

        // Then it indicates failure.
        assert!(!result.success);
        assert!(result.content.contains("No edits provided"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_error_on_bad_json() {
        // Given an edit call with invalid JSON.
        let call = ToolCall {
            id: "call_2".to_owned(),
            name: "edit".to_owned(),
            arguments: "not json".to_owned(),
        };

        // When executing the edit tool.
        let result = execute(call, test_ctx()).await;

        // Then the result indicates failure.
        assert!(!result.success);
        assert!(result.content.contains("failed to parse arguments"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_error_on_not_found() {
        // Given a temp file with content.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").expect("write temp file");

        let call = ToolCall {
            id: "call_3".to_owned(),
            name: "edit".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "edits": [{"op": "replace_text", "oldText": "missing", "newText": "replacement"}]
            })
            .to_string(),
        };

        // When executing with a missing text.
        let result = execute(call, test_ctx()).await;

        // Then the result indicates failure.
        assert!(!result.success);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_replace_text_succeeds() {
        // Given a temp file with content.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").expect("write temp file");

        let call = ToolCall {
            id: "call_4".to_owned(),
            name: "edit".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "edits": [{"op": "replace_text", "oldText": "world", "newText": "rust"}]
            })
            .to_string(),
        };

        // When executing the edit tool.
        let result = execute(call, test_ctx()).await;

        // Then the edit is applied.
        assert!(result.success, "expected success, got: {}", result.content);
        assert_eq!(
            std::fs::read_to_string(&file_path).expect("read file"),
            "hello rust"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_resolves_relative_path() {
        // Given a temp directory as CWD with a file.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").expect("write temp file");

        let call = ToolCall {
            id: "call_5".to_owned(),
            name: "edit".to_owned(),
            arguments: serde_json::json!({
                "path": "test.txt",
                "edits": [{"op": "replace_text", "oldText": "world", "newText": "rust"}]
            })
            .to_string(),
        };

        // When executing with a relative path.
        let result = execute(call, test_ctx_with_cwd(dir.path().to_owned())).await;

        // Then the edit is applied via CWD resolution.
        assert!(result.success, "expected success, got: {}", result.content);
        assert_eq!(
            std::fs::read_to_string(&file_path).expect("read file"),
            "hello rust"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_preserves_crlf() {
        // Given a temp file with CRLF line endings.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "line1\r\nline2\r\nline3\r\n").expect("write temp file");

        let call = ToolCall {
            id: "call_6".to_owned(),
            name: "edit".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "edits": [{"op": "replace_text", "oldText": "line2", "newText": "modified"}]
            })
            .to_string(),
        };

        // When executing the edit tool.
        let result = execute(call, test_ctx()).await;

        // Then CRLF line endings are preserved.
        assert!(result.success, "expected success, got: {}", result.content);
        let content = std::fs::read_to_string(&file_path).expect("read file");
        assert_eq!(content, "line1\r\nmodified\r\nline3\r\n");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_preserves_bom() {
        // Given a temp file with a UTF-8 BOM.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "\u{feff}hello world").expect("write temp file");

        let call = ToolCall {
            id: "call_7".to_owned(),
            name: "edit".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "edits": [{"op": "replace_text", "oldText": "world", "newText": "rust"}]
            })
            .to_string(),
        };

        // When executing the edit tool.
        let result = execute(call, test_ctx()).await;

        // Then the BOM is preserved.
        assert!(result.success, "expected success, got: {}", result.content);
        let content = std::fs::read_to_string(&file_path).expect("read file");
        assert_eq!(content, "\u{feff}hello rust");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_anchors_on_success() {
        // Given a temp file with content.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "alpha\nbeta\ngamma\n").expect("write temp file");

        let call = ToolCall {
            id: "call_8".to_owned(),
            name: "edit".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "edits": [{"op": "replace_text", "oldText": "beta", "newText": "BETA"}]
            })
            .to_string(),
        };

        // When executing the edit tool.
        let result = execute(call, test_ctx()).await;

        // Then the response contains an anchor block.
        assert!(result.success, "expected success, got: {}", result.content);
        assert!(
            result.content.contains("--- Anchors"),
            "expected anchor block, got: {}",
            result.content
        );
        assert!(result.content.contains("|BETA"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_stale_anchor_rejected() {
        // Given a temp file with content.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "alpha\nbeta\ngamma\n").expect("write temp file");

        // Use a stale anchor (wrong hash).
        let call = ToolCall {
            id: "call_9".to_owned(),
            name: "edit".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "edits": [{"op": "replace", "pos": "2#XX", "lines": ["replaced"]}]
            })
            .to_string(),
        };

        // When executing with a stale anchor.
        let result = execute(call, test_ctx()).await;

        // Then the result indicates failure with a stale anchor error.
        assert!(!result.success);
        assert!(result.content.contains("E_STALE_ANCHOR"));
        assert!(result.content.contains(">>>"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_unknown_op_rejected() {
        // Given a temp file with content.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").expect("write temp file");

        let call = ToolCall {
            id: "call_10".to_owned(),
            name: "edit".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "edits": [{"op": "invalid_op"}]
            })
            .to_string(),
        };

        // When executing with an unknown op.
        let result = execute(call, test_ctx()).await;

        // Then the result indicates failure.
        assert!(!result.success);
        assert!(result.content.contains("Unknown edit op"));
    }
}
