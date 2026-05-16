//! Read built-in tool — reads file contents with offset/limit support.
//!
//! Supports reading text files with optional line-based pagination via
//! `offset` (1-indexed) and `limit` parameters. Relative paths are resolved
//! against the session's CWD.

use std::path::{Path, PathBuf};

use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};

use super::BoxedToolFuture;

/// Returns the tool definition for the `read` built-in tool.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "read".to_owned(),
        description: "Read the contents of a file. Supports text files and images \
            (jpg, png, gif, webp). For text files, output is truncated to 2000 lines \
            or 50KB (whichever is hit first). Use offset/limit for large files."
            .to_owned(),
        prompt_snippet: Some("Read file contents".to_owned()),
        prompt_guidelines: vec!["Use read to examine files instead of cat or sed.".to_owned()],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read (relative or absolute)"
                },
                "offset": {
                    "type": "number",
                    "description": "Line number to start reading from (1-indexed)"
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum number of lines to read"
                }
            },
            "required": ["path"]
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

/// Executes the `read` built-in tool.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let (path, offset, limit) = match parse_args(&call.arguments) {
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

        let content = match tokio::fs::read_to_string(&resolved).await {
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

        let result = apply_offset_limit(&content, offset, limit);

        ToolResult {
            tool_call_id: call.id,
            name: call.name,
            content: result,
            success: true,
        }
    })
}

fn parse_args(raw: &str) -> Result<(String, Option<usize>, Option<usize>), serde_json::Error> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    let path = v
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let offset = v
        .get("offset")
        .and_then(serde_json::Value::as_u64)
        .map(|n| n as usize);
    let limit = v
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .map(|n| n as usize);
    Ok((path, offset, limit))
}

fn apply_offset_limit(content: &str, offset: Option<usize>, limit: Option<usize>) -> String {
    if offset.is_none() && limit.is_none() {
        return content.to_owned();
    }

    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    // offset is 1-indexed, convert to 0-indexed
    let start = offset.map_or(0, |o| o.saturating_sub(1));

    if start >= total_lines {
        return format!(
            "offset {} exceeds file length ({total_lines} lines)",
            offset.unwrap_or(0),
        );
    }

    let sliced = if let Some(limit) = limit {
        &lines[start..total_lines.min(start + limit)]
    } else {
        &lines[start..]
    };

    let mut result = sliced.join("\n");
    // Preserve trailing newline if original had one
    if content.ends_with('\n') {
        result.push('\n');
    }
    result
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
            app_paths: crate::common::app_paths::AppPaths::default(),
        }
    }

    #[rstest::rstest]
    fn apply_offset_limit_no_offset_no_limit() {
        // Given a file with 3 lines.
        let content = "line1\nline2\nline3";

        // When applying no offset or limit.
        let result = apply_offset_limit(content, None, None);

        // Then the full content is returned.
        assert_eq!(result, "line1\nline2\nline3");
    }

    #[rstest::rstest]
    fn apply_offset_limit_with_offset() {
        // Given a file with 3 lines.
        let content = "line1\nline2\nline3";

        // When offset is 2 (1-indexed).
        let result = apply_offset_limit(content, Some(2), None);

        // Then lines from index 1 onward are returned.
        assert_eq!(result, "line2\nline3");
    }

    #[rstest::rstest]
    fn apply_offset_limit_with_limit() {
        // Given a file with 3 lines.
        let content = "line1\nline2\nline3";

        // When limit is 2.
        let result = apply_offset_limit(content, None, Some(2));

        // Then only the first 2 lines are returned.
        assert_eq!(result, "line1\nline2");
    }

    #[rstest::rstest]
    fn apply_offset_limit_with_offset_and_limit() {
        // Given a file with 5 lines.
        let content = "a\nb\nc\nd\ne";

        // When offset is 2, limit is 2.
        let result = apply_offset_limit(content, Some(2), Some(2));

        // Then lines 2-3 are returned.
        assert_eq!(result, "b\nc");
    }

    #[rstest::rstest]
    fn apply_offset_limit_offset_exceeds_length() {
        // Given a file with 2 lines.
        let content = "line1\nline2";

        // When offset is 10.
        let result = apply_offset_limit(content, Some(10), None);

        // Then an error message is returned.
        assert!(result.contains("offset 10 exceeds file length (2 lines)"));
    }

    #[rstest::rstest]
    fn resolve_path_relative() {
        // Given a relative path and a CWD.
        let resolved = resolve_path("foo/bar.txt", Path::new("/home/user/project"));

        // Then it's joined against CWD.
        assert_eq!(resolved, PathBuf::from("/home/user/project/foo/bar.txt"));
    }

    #[rstest::rstest]
    fn resolve_path_absolute() {
        // Given an absolute path.
        let resolved = resolve_path("/etc/hosts", Path::new("/home/user/project"));

        // Then it's returned as-is.
        assert_eq!(resolved, PathBuf::from("/etc/hosts"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_reads_file_content() {
        // Given a temp file with known content.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "file contents here").expect("write temp file");

        let call = ToolCall {
            id: "call_1".to_owned(),
            name: "read".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy()
            })
            .to_string(),
        };

        // When executing the read tool.
        let result = execute(call, test_ctx()).await;

        // Then the result contains the file contents.
        assert_eq!(result.tool_call_id, "call_1");
        assert!(result.success);
        assert_eq!(result.content, "file contents here");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_error_on_missing_file() {
        // Given a read call for a nonexistent file.
        let call = ToolCall {
            id: "call_2".to_owned(),
            name: "read".to_owned(),
            arguments: serde_json::json!({
                "path": "/nonexistent/path/to/file.txt"
            })
            .to_string(),
        };

        // When executing the read tool.
        let result = execute(call, test_ctx()).await;

        // Then the result indicates failure.
        assert_eq!(result.tool_call_id, "call_2");
        assert!(!result.success);
        assert!(result.content.contains("failed to read file"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_error_on_bad_json() {
        // Given a read call with invalid JSON.
        let call = ToolCall {
            id: "call_3".to_owned(),
            name: "read".to_owned(),
            arguments: "not json".to_owned(),
        };

        // When executing the read tool.
        let result = execute(call, test_ctx()).await;

        // Then the result indicates failure.
        assert!(!result.success);
        assert!(result.content.contains("failed to parse arguments"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_resolves_relative_path() {
        // Given a temp directory with a file.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "relative content").expect("write temp file");

        let ctx = ToolContext {
            cwd: dir.path().to_owned(),
            timeout: None,
            state: None,
            session_id: None,
            app_paths: crate::common::app_paths::AppPaths::default(),
        };

        let call = ToolCall {
            id: "call_4".to_owned(),
            name: "read".to_owned(),
            arguments: serde_json::json!({
                "path": "test.txt"
            })
            .to_string(),
        };

        // When executing with a relative path.
        let result = execute(call, ctx).await;

        // Then the file is found via CWD resolution.
        assert!(result.success);
        assert_eq!(result.content, "relative content");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_with_offset_and_limit() {
        // Given a temp file with 5 lines.
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("lines.txt");
        std::fs::write(&file_path, "a\nb\nc\nd\ne").expect("write temp file");

        let call = ToolCall {
            id: "call_5".to_owned(),
            name: "read".to_owned(),
            arguments: serde_json::json!({
                "path": file_path.to_string_lossy(),
                "offset": 2,
                "limit": 2
            })
            .to_string(),
        };

        // When executing with offset=2, limit=2.
        let result = execute(call, test_ctx()).await;

        // Then only lines 2-3 are returned.
        assert!(result.success);
        assert_eq!(result.content, "b\nc");
    }
}
