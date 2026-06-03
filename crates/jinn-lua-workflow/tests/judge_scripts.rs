//! End-to-end tests for the judge workflow scripts.
//!
//! Loads the actual Lua files from `res/plugins/judge_fail` and
//! `res/plugins/judge_pass`, runs them through `spawn_one_shot`,
//! and verifies the correct host requests are sent.

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unwrap_in_result,
        reason = "test code, panics are acceptable"
    )]

    use std::path::PathBuf;

    use jinn_lua_workflow::{CtxConfig, HostRequest, spawn_one_shot};

    fn read_script(name: &str) -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("res/plugins")
            .join(name)
            .join("init.lua");
        std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!("failed to read {}: {e}", path.display());
        })
    }

    fn judge_fail_config() -> CtxConfig {
        CtxConfig::data_only(&serde_json::json!({}))
            .with_push_user()
            .session_id("test-session".to_owned())
    }

    fn judge_pass_config() -> CtxConfig {
        CtxConfig::data_only(&serde_json::json!({}))
            .with_push_system()
            .with_turn_off()
            .session_id("test-session".to_owned())
            .workflow_id("test-workflow".to_owned())
    }

    #[tokio::test]
    async fn judge_fail_pushes_user_message() {
        let (host_tx, host_rx) = kanal::unbounded::<HostRequest>();
        let script = read_script("judge_fail");

        let handle = spawn_one_shot(
            script,
            "judge_fail".to_owned(),
            host_tx,
            judge_fail_config(),
        );

        // Receive and verify the PushUser request.
        let req = host_rx.recv().expect("should receive request");
        match req {
            HostRequest::PushUser {
                session_id,
                text,
                respond_to,
            } => {
                assert_eq!(session_id, "test-session");
                assert_eq!(text, "judgement failed, try again");
                respond_to.send(Ok(())).expect("respond");
            }
            other => panic!("expected PushUser, got {other}"),
        }

        // Verify the script completes successfully.
        let result = handle.await.expect("task join").expect("inner result");
        assert!(result.is_null());
    }

    #[tokio::test]
    async fn judge_pass_pushes_system_and_turns_off() {
        let (host_tx, host_rx) = kanal::unbounded::<HostRequest>();
        let script = read_script("judge_pass");

        let handle = spawn_one_shot(
            script,
            "judge_pass".to_owned(),
            host_tx,
            judge_pass_config(),
        );

        // First request: PushSystem.
        let req1 = host_rx.recv().expect("should receive first request");
        match req1 {
            HostRequest::PushSystem {
                session_id,
                text,
                respond_to,
            } => {
                assert_eq!(session_id, "test-session");
                assert_eq!(text, "judgement passed");
                respond_to.send(Ok(())).expect("respond");
            }
            other => panic!("expected PushSystem, got {other}"),
        }

        // Second request: TurnOff.
        let req2 = host_rx.recv().expect("should receive second request");
        match req2 {
            HostRequest::TurnOff {
                workflow_id,
                respond_to,
            } => {
                assert_eq!(workflow_id, "test-workflow");
                respond_to.send(Ok(())).expect("respond");
            }
            other => panic!("expected TurnOff, got {other}"),
        }

        // Verify the script completes successfully.
        let result = handle.await.expect("task join").expect("inner result");
        assert!(result.is_null());
    }

    #[tokio::test]
    async fn judge_fail_script_is_valid_lua() {
        let script = read_script("judge_fail");
        // Parse check: should not contain obvious syntax errors.
        assert!(script.contains("return"));
        assert!(script.contains("run = function(ctx)"));
        assert!(script.contains("push_user"));
    }

    #[tokio::test]
    async fn judge_pass_script_is_valid_lua() {
        let script = read_script("judge_pass");
        assert!(script.contains("return"));
        assert!(script.contains("run = function(ctx)"));
        assert!(script.contains("push_system"));
        assert!(script.contains("turn_off"));
    }
}
