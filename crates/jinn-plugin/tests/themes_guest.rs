//! End-to-end host↔guest test against the real wasmtime engine using the
//! real first-party themes guest.
//!
//! Requires the guest artifact `target/wasm32-wasip2/release/theme-loader.wasm`
//! (built by `just build-plugins`). When the artifact is absent the tests
//! are skipped with a note — CI runs `just build-plugins` first.

#![allow(clippy::expect_used, reason = "test code")]
#![allow(clippy::panic, reason = "test code")]
#![allow(clippy::print_stderr, reason = "skip notice")]

use std::path::PathBuf;
use std::time::Duration;

use jinn_plugin::{Grants, PluginEngine, PluginHost};
use jinn_plugin_api::{HostToPlugin, PROTOCOL_VERSION, PluginToHost};

/// The workspace root, from the test crate's manifest dir.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Locates the built themes guest, or `None` when not built.
fn themes_wasm() -> Option<PathBuf> {
    let path = workspace_root().join("target/wasm32-wasip2/release/theme-loader.wasm");
    path.exists().then_some(path)
}

/// Grants a themes-shaped plugin: read access to the workspace res/themes.
fn themes_grants() -> Grants {
    Grants {
        read_dirs: vec![workspace_root().join("res/themes")],
        write_dirs: vec![],
        http: false,
        config: serde_json::Value::Null,
    }
}

/// The full guest lifecycle over the real engine: Hello → Welcome →
/// SetThemeEntries → guest end.
#[rstest::rstest]
#[tokio::test]
async fn themes_guest_handshakes_and_contributes_over_real_engine() {
    let Some(wasm) = themes_wasm() else {
        eprintln!("skipping: theme-loader.wasm not built (run `just build-plugins`)");
        return;
    };

    // Given a real host for the themes guest.
    let engine = PluginEngine::new().expect("engine");
    let mut host =
        PluginHost::start(&engine, "theme-loader", &wasm, &themes_grants()).expect("guest start");

    // When the handshake completes and the guest runs to completion.
    let mut reader = host.split();

    let read = tokio::time::timeout(Duration::from_secs(20), reader.read_next())
        .await
        .expect("hello within timeout")
        .expect("read ok");
    let Some(hello) = read else {
        let stderr = host.stderr_tail().await;
        panic!("no hello; stderr: {stderr}");
    };
    let jinn_plugin_api::PluginToHostOrHostToPlugin::Plugin(PluginToHost::Hello(hello)) = hello.msg
    else {
        panic!("first message was not Hello");
    };
    assert_eq!(hello.protocol_version, PROTOCOL_VERSION);

    let welcome = jinn_plugin_api::Envelope::for_host(
        HostToPlugin::Welcome(jinn_plugin_api::Welcome {
            protocol_version: PROTOCOL_VERSION,
            plugin_id: "theme-loader".to_owned(),
            read_dirs: themes_grants()
                .read_dirs
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            write_dirs: vec![],
            http_allowed: false,
            config: serde_json::Value::Null,
        }),
        0,
        0,
    );
    host.write(&welcome).await.expect("welcome write");

    // Then the guest contributes its theme set and ends cleanly.
    let mut names = Vec::new();
    loop {
        let next = tokio::time::timeout(Duration::from_secs(20), reader.read_next())
            .await
            .expect("contribution within timeout")
            .expect("read ok");
        let Some(envelope) = next else { break };
        if let jinn_plugin_api::PluginToHostOrHostToPlugin::Plugin(PluginToHost::SetThemeEntries(
            entries,
        )) = envelope.msg
        {
            names.extend(entries.themes.iter().map(|t| t.name.clone()));
        }
    }

    assert!(
        names.iter().all(|n| n != "default") || names.contains(&"default".to_owned()),
        "sanity"
    );
    assert!(
        names.contains(&"default".to_owned()),
        "the granted res/themes dir contains default.toml: {names:?}"
    );
    assert!(
        names.contains(&"nord-light".to_owned()),
        "the new theme is carried: {names:?}"
    );
    assert!(
        host.is_finished(),
        "guest task completed after its final contribution"
    );
}

/// A guest with no granted dirs contributes an empty set and ends.
#[rstest::rstest]
#[tokio::test]
async fn themes_guest_with_empty_grants_contributes_nothing() {
    let Some(wasm) = themes_wasm() else {
        eprintln!("skipping: theme-loader.wasm not built (run `just build-plugins`)");
        return;
    };

    // Given a host granting no directories.
    let engine = PluginEngine::new().expect("engine");
    let mut host = PluginHost::start(
        &engine,
        "theme-loader",
        &wasm,
        &Grants {
            read_dirs: vec![],
            write_dirs: vec![],
            http: false,
            config: serde_json::Value::Null,
        },
    )
    .expect("guest start");
    let mut reader = host.split();

    // When the handshake completes.
    let hello = reader.read_next().await.expect("read ok").expect("hello");
    let _ = hello;
    let welcome = jinn_plugin_api::Envelope::for_host(
        HostToPlugin::Welcome(jinn_plugin_api::Welcome {
            protocol_version: PROTOCOL_VERSION,
            plugin_id: "theme-loader".to_owned(),
            read_dirs: vec![],
            write_dirs: vec![],
            http_allowed: false,
            config: serde_json::Value::Null,
        }),
        0,
        0,
    );
    host.write(&welcome).await.expect("welcome write");

    // Then the single contribution carries no themes and the guest ends.
    let mut contributed_any = false;
    loop {
        let Some(envelope) = reader.read_next().await.expect("read ok") else {
            break;
        };
        if let jinn_plugin_api::PluginToHostOrHostToPlugin::Plugin(PluginToHost::SetThemeEntries(
            entries,
        )) = envelope.msg
        {
            contributed_any = true;
            assert!(entries.themes.is_empty(), "no granted dirs");
        }
    }
    assert!(contributed_any, "guest still reported its (empty) set");
}
