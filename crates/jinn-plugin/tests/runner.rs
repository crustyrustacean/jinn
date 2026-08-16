//! Runner tests with a fake guest process.
//!
//! The real runner child is `jinn --serve-wasm-plugin <wasm>`; these tests
//! substitute a small shell script that speaks the same NDJSON framing,
//! proving the host side: spawn wiring, framing round-trips, malformed-line
//! dropping, the oversized-line cap, and bounded shutdown.
//! dropping, the oversized-line cap, and bounded shutdown.

#![allow(
    clippy::expect_used,
    clippy::unreachable,
    reason = "test-only fake-guest fixtures"
)]
use std::path::PathBuf;
use std::time::Duration;

use jinn_plugin::{DirContext, Grants, PathGrant, PluginProcess, resolve_grants};

/// Writes a fake guest script that echoes envelope behavior.
///
/// Modes:
/// - `echo`: reads lines, prefixes `msg.type` with `"ack: "`, writes back
/// - `garbage`: writes a malformed line first, then a good one
/// - `huge`: writes a line over the byte cap
/// - `silent`: writes nothing, never exits
fn fake_guest(mode: &str) -> PathBuf {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("guest.sh");
    let body = match mode {
        "echo" => {
            r#"#!/bin/sh
while IFS= read -r line; do
  printf '%s\n' "$line"
done
"#
        }
        "garbage" => {
            r#"#!/bin/sh
printf 'this is not json\n'
printf '{"v":1,"seq":1,"ts":0,"type":"hello","protocol_version":1,"name":"t"}\n'
"#
        }
        "huge" => {
            let mut body = String::from("#!/bin/sh\nprintf '");
            body.push_str(&"x".repeat(1024 * 1024 + 10));
            body.push_str("'\n");
            body.leak()
        }
        "silent" => "#!/bin/sh\nsleep 59\n",
        _ => unreachable!("unknown fake guest mode"),
    };
    std::fs::write(&path, body).expect("write script");
    make_executable(&path);
    // Leak the tempdir so the script lives as long as the test.
    std::mem::forget(dir);
    path
}

#[cfg(unix)]
fn make_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("chmod");
}

#[cfg(not(unix))]
fn make_executable(_path: &std::path::Path) {}

fn test_grants() -> Grants {
    let ctx = DirContext {
        config_dir: PathBuf::from("/tmp/cfg"),
        data_dir: PathBuf::from("/tmp/data"),
        plugin_name: "test".to_owned(),
    };
    resolve_grants(&[], false, serde_json::Value::Null, &ctx).expect("resolve grants")
}

fn envelope(seq: u64) -> jinn_plugin_api::Envelope {
    use jinn_plugin_api::{Hello, PluginToHost};
    jinn_plugin_api::Envelope {
        v: jinn_plugin_api::PROTOCOL_VERSION,
        seq,
        ts: 0,
        msg: jinn_plugin_api::Envelope::for_plugin(
            PluginToHost::Hello(Hello {
                protocol_version: jinn_plugin_api::PROTOCOL_VERSION,
                name: "test".to_owned(),
                subscriptions: vec![],
            }),
            seq,
            0,
        )
        .msg,
    }
}

#[tokio::test]
async fn write_then_read_round_trips_envelope() {
    // Given an echo fake guest.
    let guest = fake_guest("echo");
    let mut process =
        PluginProcess::spawn(&guest, "test", &guest, &test_grants()).expect("spawn fake guest");

    // When writing an envelope.
    process.write(&envelope(1)).await.expect("write");

    // Then the same envelope comes back.
    let read = tokio::time::timeout(Duration::from_secs(5), process.read())
        .await
        .expect("timeout")
        .expect("read");
    assert_eq!(read.expect("envelope").seq, 1);

    process.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn malformed_line_is_dropped_not_fatal() {
    // Given a garbage-then-good fake guest.
    let guest = fake_guest("garbage");
    let mut process =
        PluginProcess::spawn(&guest, "test", &guest, &test_grants()).expect("spawn fake guest");

    // When reading while the guest emits a malformed line first.
    let read = tokio::time::timeout(Duration::from_secs(5), process.read())
        .await
        .expect("timeout")
        .expect("read");

    // Then the malformed line was skipped and the good envelope arrives.
    assert_eq!(read.expect("envelope").seq, 1);

    process.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn oversized_line_is_dropped() {
    // Given a guest that writes past the byte cap.
    let guest = fake_guest("huge");
    let mut process =
        PluginProcess::spawn(&guest, "test", &guest, &test_grants()).expect("spawn fake guest");

    // When reading after the huge line.
    // The read skips it and returns None at EOF (nothing else follows).
    let read = tokio::time::timeout(Duration::from_secs(5), process.read())
        .await
        .expect("timeout")
        .expect("read");

    // Then the oversized line was dropped: EOF, not an envelope.
    assert!(read.is_none());

    process.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn read_returns_none_at_eof() {
    // Given a guest that writes one line and exits.
    let guest = fake_guest("garbage"); // writes then exits

    let mut process =
        PluginProcess::spawn(&guest, "test", &guest, &test_grants()).expect("spawn fake guest");

    // When reading past the guest's output.
    let first = tokio::time::timeout(Duration::from_secs(5), process.read())
        .await
        .expect("timeout 1")
        .expect("read 1")
        .expect("envelope 1");
    let second = tokio::time::timeout(Duration::from_secs(5), process.read())
        .await
        .expect("timeout 2")
        .expect("read 2");

    // Then the first read yielded the envelope and the second hit EOF.
    assert_eq!(first.seq, 1);
    assert!(second.is_none());
}

#[tokio::test]
async fn shutdown_forces_silent_guest_exit() {
    // Given a guest that never exits on its own.
    let guest = fake_guest("silent");
    let mut process =
        PluginProcess::spawn(&guest, "test", &guest, &test_grants()).expect("spawn fake guest");

    // When shutting down with a bounded wait.
    let started = std::time::Instant::now();
    process.shutdown().await.expect("shutdown");

    // Then the force-kill path fires: bounded by the 5s timeout plus slack
    // for the kill syscall itself.
    assert!(started.elapsed() < Duration::from_secs(6));
}

#[test]
fn resolve_grants_always_includes_scratch_dir() {
    // Given no manifest grants and a directory context.
    let ctx = DirContext {
        config_dir: PathBuf::from("/cfg"),
        data_dir: PathBuf::from("/data"),
        plugin_name: "p".to_owned(),
    };

    // When resolving grants with an empty manifest list.
    let grants = resolve_grants(&[], true, serde_json::Value::Null, &ctx).expect("resolve");

    // Then the writable set is exactly the scratch dir and http is granted.
    assert_eq!(grants.write_dirs, vec![PathBuf::from("/data/plugins/p")]);
    assert!(grants.http);
}

#[test]
fn resolve_grants_expands_templates_in_order() {
    // Given a manifest with a read and a writable grant.
    let grants = [
        PathGrant {
            path: "<config_dir>/themes".to_owned(),
            writable: false,
        },
        PathGrant {
            path: "<data_dir>/notes".to_owned(),
            writable: true,
        },
    ];
    let ctx = DirContext {
        config_dir: PathBuf::from("/cfg"),
        data_dir: PathBuf::from("/data"),
        plugin_name: "p".to_owned(),
    };

    // When resolving.
    let resolved = resolve_grants(&grants, false, serde_json::Value::Null, &ctx).expect("resolve");

    // Then templates expanded to the right dirs with the right intent.
    assert_eq!(resolved.read_dirs, vec![PathBuf::from("/cfg/themes")]);
    assert!(resolved.write_dirs.contains(&PathBuf::from("/data/notes")));
}

#[test]
fn resolve_grants_rejects_unknown_variable() {
    // Given a manifest using an undefined variable.
    let grants = [PathGrant {
        path: "<root>/secrets".to_owned(),
        writable: false,
    }];
    let ctx = DirContext {
        config_dir: PathBuf::from("/cfg"),
        data_dir: PathBuf::from("/data"),
        plugin_name: "p".to_owned(),
    };

    // When resolving.
    let result = resolve_grants(&grants, false, serde_json::Value::Null, &ctx);

    // Then it fails with UnknownVariable.
    assert!(result.is_err());
}
