use std::path::PathBuf;

use crate::feat::tools_actor::builtin_read;
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext};

fn test_ctx() -> ToolContext {
    ToolContext {
        cwd: PathBuf::from("/tmp"),
        timeout: None,
        state: None,
        session_id: None,
        app_paths: crate::common::app_paths::AppPaths::default(),
        sink: None,
        max_output_lines: None,
        max_output_bytes: None,
    }
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
    let result = builtin_read::execute(call, test_ctx()).await;

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
    let result = builtin_read::execute(call, test_ctx()).await;

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
    let result = builtin_read::execute(call, test_ctx()).await;

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
        sink: None,
        max_output_lines: None,
        max_output_bytes: None,
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
    let result = builtin_read::execute(call, ctx).await;

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
    let result = builtin_read::execute(call, test_ctx()).await;

    // Then only lines 2-3 are returned.
    assert!(result.success);
    assert_eq!(result.content, "b\nc");
}
