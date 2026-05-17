use crate::feat::tools_actor::edit::execute;
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext};
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
        app_paths: crate::common::app_paths::AppPaths::default(),
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
