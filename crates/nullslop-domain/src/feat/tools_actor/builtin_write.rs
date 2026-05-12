//! Write built-in tool — writes content to a file.
//!
//! Creates the file if it doesn't exist, overwrites if it does.
//! Automatically creates parent directories. Relative paths are resolved
//! against the session's CWD.

use std::path::{Path, PathBuf};

use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};

use super::BoxedToolFuture;

/// Returns the tool definition for the `write` built-in tool.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "write".to_owned(),
        description: "Write content to a file. Creates the file if it doesn't exist, \
            overwrites if it does. Automatically creates parent directories."
            .to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write (relative or absolute)"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
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

/// Executes the `write` built-in tool.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let (path, content) = match parse_args(&call.arguments) {
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

        let resolved = resolve_path(&path, &ctx.cwd);

        if let Some(parent) = resolved.parent()
            && !parent.as_os_str().is_empty()
            && let Err(e) = tokio::fs::create_dir_all(parent).await
        {
            return ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: format!(
                    "failed to create parent directories for '{}': {e}",
                    resolved.display()
                ),
                success: false,
            };
        }

        match tokio::fs::write(&resolved, &content).await {
            Ok(()) => ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: format!("wrote {} bytes to {}", content.len(), resolved.display()),
                success: true,
            },
            Err(e) => ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: format!("failed to write file '{}': {e}", resolved.display()),
                success: false,
            },
        }
    })
}

fn parse_args(raw: &str) -> Result<(String, String), serde_json::Error> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    let path = v
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let content = v
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    Ok((path, content))
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
    async fn execute_writes_file_content() {
        // Given a temp directory.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("output.txt");

        let call = ToolCall {
            id: "call_1".to_owned(),
            name: "write".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "content": "hello from write"
            })
            .to_string(),
        };

        // When executing the write tool.
        let result = execute(call, test_ctx()).await;

        // Then the result indicates success.
        assert_eq!(result.tool_call_id, "call_1");
        assert!(result.success, "expected success, got: {}", result.content);
        assert!(result.content.contains("wrote 16 bytes"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_creates_file_with_content() {
        // Given a temp directory.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("output.txt");

        let call = ToolCall {
            id: "call_1".to_owned(),
            name: "write".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "content": "hello from write"
            })
            .to_string(),
        };

        // When executing the write tool.
        let _result = execute(call, test_ctx()).await;

        // Then the file contains the written content.
        let content = std::fs::read_to_string(&file_path).expect("read written file");
        assert_eq!(content, "hello from write");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_creates_parent_dirs() {
        // Given a temp directory with a nested path.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("nested").join("deep").join("file.txt");

        let call = ToolCall {
            id: "call_2".to_owned(),
            name: "write".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "content": "nested content"
            })
            .to_string(),
        };

        // When executing the write tool.
        let result = execute(call, test_ctx()).await;

        // Then the result indicates success.
        assert!(result.success, "expected success, got: {}", result.content);

        // And the file was created with parent directories.
        let content = std::fs::read_to_string(&file_path).expect("read written file");
        assert_eq!(content, "nested content");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_overwrites_existing_file() {
        // Given a temp file with existing content.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("existing.txt");
        std::fs::write(&file_path, "old content").expect("write existing file");

        let call = ToolCall {
            id: "call_3".to_owned(),
            name: "write".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "content": "new content"
            })
            .to_string(),
        };

        // When executing the write tool.
        let result = execute(call, test_ctx()).await;

        // Then the result indicates success.
        assert!(result.success);

        // And the file was overwritten.
        let content = std::fs::read_to_string(&file_path).expect("read overwritten file");
        assert_eq!(content, "new content");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_error_on_bad_json() {
        // Given a write call with invalid JSON.
        let call = ToolCall {
            id: "call_4".to_owned(),
            name: "write".to_owned(),
            arguments: "not json".to_owned(),
        };

        // When executing the write tool.
        let result = execute(call, test_ctx()).await;

        // Then the result indicates failure.
        assert!(!result.success);
        assert!(result.content.contains("failed to parse arguments"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_resolves_relative_path() {
        // Given a temp directory as CWD.
        let dir = tempfile::tempdir().expect("create temp dir");
        let ctx = ToolContext {
            cwd: dir.path().to_owned(),
            timeout: None,
    state: None,
    session_id: None,
        };

        let call = ToolCall {
            id: "call_5".to_owned(),
            name: "write".to_owned(),
            arguments: serde_json::json!({
                "path": "relative.txt",
                "content": "relative content"
            })
            .to_string(),
        };

        // When executing with a relative path.
        let result = execute(call, ctx).await;

        // Then the file is created via CWD resolution.
        assert!(result.success);
        let file_path = dir.path().join("relative.txt");
        let content = std::fs::read_to_string(&file_path).expect("read written file");
        assert_eq!(content, "relative content");
    }
}
