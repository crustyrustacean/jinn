//! Edit built-in tool — performs exact text replacement in files.
//!
//! Supports multiple non-overlapping edits per call, BOM preservation,
//! line-ending preservation, and unified diff output.

mod diff;
mod line_ending;

use std::path::{Path, PathBuf};

use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};

use super::BoxedToolFuture;
use diff::Edit;

// pub use diff::generate_unified_diff; // Not needed outside the module yet
/// Returns the tool definition for the `edit` built-in tool.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "edit".to_owned(),
        description: "Edit a file using exact text replacement. \
            Each oldText must match a unique, non-overlapping region of the original file. \
            Returns a unified diff of the changes."
            .to_owned(),
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
                            "oldText": {
                                "type": "string",
                                "description": "Exact text to find in the file (must be unique)"
                            },
                            "newText": {
                                "type": "string",
                                "description": "Replacement text"
                            }
                        },
                        "required": ["oldText", "newText"],
                        "additionalProperties": false
                    },
                    "description": "One or more text replacements to apply"
                }
            },
            "required": ["path", "edits"],
            "additionalProperties": false
        }),
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
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let (path, edits) = match parse_args(&call.arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: format!("failed to parse arguments: {e}"),
                    success: false,
                };
            }
        };

        if edits.is_empty() {
            return ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: "no edits provided".to_owned(),
                success: false,
            };
        }

        let resolved = resolve_path(&path, &ctx.cwd);

        // Read file
        let raw_content = match tokio::fs::read_to_string(&resolved).await {
            Ok(c) => c,
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: format!("failed to read file '{}': {e}", resolved.display()),
                    success: false,
                };
            }
        };

        // Strip BOM and detect line endings
        let (content, bom) = line_ending::strip_bom(&raw_content);
        let detected_ending = line_ending::detect_line_ending(content);

        // Normalize to LF for matching
        let normalized = line_ending::normalize_to_lf(content);

        // Find and validate edits against normalized content
        let matched = match diff::find_and_validate_edits(&normalized, &edits) {
            Ok(m) => m,
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: format!("edit validation failed: {e}"),
                    success: false,
                };
            }
        };

        // Apply edits
        let modified_normalized = diff::apply_edits(&normalized, &matched);

        // Restore line endings
        let modified = line_ending::restore_line_endings(&modified_normalized, detected_ending);

        // Re-add BOM if present
        let final_content = match bom {
            Some(b) => format!("{b}{modified}"),
            None => modified,
        };

        // Generate diff against normalized content for consistent output
        let diff = diff::generate_unified_diff(&normalized, &modified_normalized, &path);

        // Write back
        if let Err(e) = tokio::fs::write(&resolved, &final_content).await {
            return ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: format!("failed to write file '{}': {e}", resolved.display()),
                success: false,
            };
        }

        ToolResult {
            tool_call_id: call.id,
            name: call.name,
            content: format!(
                "Successfully replaced {} block(s) in {}.\n{}",
                matched.len(),
                resolved.display(),
                diff
            ),
            success: true,
        }
    })
}

/// Parses the arguments from the tool call JSON.
fn parse_args(raw: &str) -> Result<(String, Vec<Edit>), serde_json::Error> {
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
                    let old_text = item.get("oldText")?.as_str()?.to_owned();
                    let new_text = item.get("newText")?.as_str()?.to_owned();
                    Some(Edit { old_text, new_text })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok((path, edits))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_ctx() -> ToolContext {
        ToolContext {
            cwd: PathBuf::from("/tmp"),
            timeout: None,
            state: None,
            session_id: None,
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_single_edit() {
        // Given a temp file with content.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").expect("write temp file");

        let call = ToolCall {
            id: "call_1".to_owned(),
            name: "edit".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "edits": [{"oldText": "world", "newText": "rust"}]
            })
            .to_string(),
        };

        // When executing the edit tool.
        let result = execute(call, test_ctx()).await;

        // Then the edit is applied.
        assert!(result.success, "expected success, got: {}", result.content);
        assert!(result.content.contains("Successfully replaced 1 block(s)"));
        assert_eq!(
            std::fs::read_to_string(&file_path).expect("read file"),
            "hello rust"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_multiple_edits() {
        // Given a temp file with content.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "aaa bbb ccc").expect("write temp file");

        let call = ToolCall {
            id: "call_2".to_owned(),
            name: "edit".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "edits": [
                    {"oldText": "aaa", "newText": "xxx"},
                    {"oldText": "ccc", "newText": "zzz"}
                ]
            })
            .to_string(),
        };

        // When executing the edit tool.
        let result = execute(call, test_ctx()).await;

        // Then both edits are applied.
        assert!(result.success, "expected success, got: {}", result.content);
        assert!(result.content.contains("Successfully replaced 2 block(s)"));
        assert_eq!(
            std::fs::read_to_string(&file_path).expect("read file"),
            "xxx bbb zzz"
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
            id: "call_3".to_owned(),
            name: "edit".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "edits": [{"oldText": "line2", "newText": "modified"}]
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
            id: "call_4".to_owned(),
            name: "edit".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "edits": [{"oldText": "world", "newText": "rust"}]
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
    async fn execute_returns_error_on_not_found() {
        // Given a temp file with content.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").expect("write temp file");

        let call = ToolCall {
            id: "call_5".to_owned(),
            name: "edit".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "edits": [{"oldText": "missing", "newText": "replacement"}]
            })
            .to_string(),
        };

        // When executing the edit tool.
        let result = execute(call, test_ctx()).await;

        // Then the result indicates failure.
        assert!(!result.success);
        assert!(result.content.contains("not found"));

        // And the file is unchanged.
        assert_eq!(
            std::fs::read_to_string(&file_path).expect("read file"),
            "hello world"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_error_on_duplicate_match() {
        // Given a temp file with a repeated substring.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "foo bar foo").expect("write temp file");

        let call = ToolCall {
            id: "call_6".to_owned(),
            name: "edit".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "edits": [{"oldText": "foo", "newText": "baz"}]
            })
            .to_string(),
        };

        // When executing the edit tool.
        let result = execute(call, test_ctx()).await;

        // Then the result indicates failure.
        assert!(!result.success);
        assert!(result.content.contains("found 2 times"));

        // And the file is unchanged.
        assert_eq!(
            std::fs::read_to_string(&file_path).expect("read file"),
            "foo bar foo"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_error_on_bad_json() {
        // Given an edit call with invalid JSON.
        let call = ToolCall {
            id: "call_7".to_owned(),
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
    async fn execute_resolves_relative_path() {
        // Given a temp directory as CWD with a file.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").expect("write temp file");

        let ctx = ToolContext {
            cwd: dir.path().to_owned(),
            timeout: None,
            state: None,
            session_id: None,
        };

        let call = ToolCall {
            id: "call_8".to_owned(),
            name: "edit".to_owned(),
            arguments: serde_json::json!({
                "path": "test.txt",
                "edits": [{"oldText": "hello", "newText": "goodbye"}]
            })
            .to_string(),
        };

        // When executing with a relative path.
        let result = execute(call, ctx).await;

        // Then the edit is applied via CWD resolution.
        assert!(result.success, "expected success, got: {}", result.content);
        assert_eq!(
            std::fs::read_to_string(&file_path).expect("read file"),
            "goodbye world"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_diff_in_output() {
        // Given a temp file with content.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "line1\nline2\nline3\n").expect("write temp file");

        let call = ToolCall {
            id: "call_9".to_owned(),
            name: "edit".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "edits": [{"oldText": "line2", "newText": "modified"}]
            })
            .to_string(),
        };

        // When executing the edit tool.
        let result = execute(call, test_ctx()).await;

        // Then the output contains a unified diff.
        assert!(result.success);
        assert!(result.content.contains("-line2"));
        assert!(result.content.contains("+modified"));
    }
}
