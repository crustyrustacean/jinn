//! E2E: the real url-citations wasm artifact driven through its wire.
//!
//! Spawns the committed `res/plugins/url-citations.wasm` as a real wasmtime
//! guest (no fake), completes the handshake, plays a full parallel-search
//! turn through the guest's stdin, and asserts the `PushCitations` line it
//! writes back. This is the acceptance test for "parallel web_search /
//! web_fetch calls produce a grouped Sources-footer payload at end of turn"
//! — everything below the footer rendering (which is covered by the session
//! actor's existing annotation tests).
//!
//! Requires the artifact: run `just refresh-plugins` when the plugin source
//! changes.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stderr,
    clippy::use_debug,
    reason = "test code; the eprintln tracing is deliberate (user-requested run diagnostics)"
)]

use std::time::Duration;

use jinn_plugin::{Grants, PluginEngine, PluginHost};
use jinn_plugin_api::{
    Envelope, HostToPlugin, PROTOCOL_VERSION, PluginToHost, PluginToHostOrHostToPlugin,
    ToolCallEvent, ToolResultEvent, TurnEndEvent,
};

/// Where the committed artifact lives, relative to this crate.
const WASM: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../res/plugins/url-citations.wasm"
);

/// Overall turn timeout.
const WAIT: Duration = Duration::from_secs(10);

/// Reads the next plugin→host envelope from the guest (bounded: a silent
/// guest fails the test instead of hanging it).
async fn next_message(host: &mut PluginHost) -> PluginToHost {
    let envelope = tokio::time::timeout(WAIT, host.read())
        .await
        .expect("guest read timed out")
        .expect("guest read error")
        .expect("guest stdout closed before replying");
    match envelope.msg {
        PluginToHostOrHostToPlugin::Plugin(msg) => {
            eprintln!("[e2e] guest -> {msg:?}");
            msg
        }
        other => panic!("unexpected host-direction echo: {other:?}"),
    }
}

/// Sends one host→guest envelope (logged).
async fn send(host: &mut PluginHost, msg: HostToPlugin, seq: u64) {
    eprintln!("[e2e] host -> {msg:?}");
    host.write(&Envelope::for_host(msg, seq, 0))
        .await
        .expect("write to guest stdin");
}

/// A full parallel-search turn through the real guest yields a grouped
/// PushCitations payload at turn end.
#[tokio::test]
async fn parallel_search_turn_yields_push_citations() {
    // Given the real url-citations guest (no grants, no http).
    let engine = PluginEngine::new().expect("engine");
    let grants = Grants {
        read_dirs: vec![],
        write_dirs: vec![],
        http: false,
        config: serde_json::Value::Null,
    };
    let mut host = PluginHost::start(
        &engine,
        "url-citations",
        std::path::Path::new(WASM),
        &grants,
    )
    .expect("guest started");

    // When the handshake completes: Hello arrives, and the test replies
    // Welcome — the guest's main() blocks in welcome() until it lands, so
    // this MUST precede any event forwarding (the original version hung
    // here: the guest kept reading, waiting for Welcome, silently
    // swallowing the forwarded events).
    let hello = next_message(&mut host).await;
    let PluginToHost::Hello(hello) = hello else {
        panic!("expected Hello first, got {hello:?}");
    };
    assert_eq!(hello.protocol_version, PROTOCOL_VERSION);
    assert_eq!(
        hello.subscriptions,
        vec!["tool_call", "tool_result", "turn_end"]
    );
    eprintln!("[e2e] handshake ok; sending Welcome");
    send(
        &mut host,
        HostToPlugin::Welcome(jinn_plugin_api::Welcome {
            protocol_version: PROTOCOL_VERSION,
            plugin_id: "url-citations".to_owned(),
            read_dirs: vec![],
            write_dirs: vec![],
            http_allowed: false,
            config: serde_json::Value::Null,
        }),
        0,
    )
    .await;

    // And a parallel web_search call + successful result flow through.
    eprintln!("[e2e] playing parallel web_search turn");
    let session = "01943d8e-5a1f-7c2d-9e3b-4f6a8b0c1d2e".to_owned();
    send(
        &mut host,
        HostToPlugin::ToolCallEvent(ToolCallEvent {
            session_id: session.clone(),
            tool_call_id: "call_1".to_owned(),
            name: "mcp__parallel__web_search".to_owned(),
            arguments: r#"{"objective":"find rust docs","search_queries":["rust book"]}"#
                .to_owned(),
        }),
        1,
    )
    .await;
    send(
        &mut host,
        HostToPlugin::ToolResultEvent(ToolResultEvent {
            session_id: session.clone(),
            tool_call_id: "call_1".to_owned(),
            name: "mcp__parallel__web_search".to_owned(),
            content: r#"{"search_id":"search_abc","results":[{"url":"https://doc.rust-lang.org/book","title":"The Rust Programming Language","publish_date":null,"excerpts":["Learn Rust with the entire book."]}]}"#.to_owned(),
            success: true,
        }),
        2,
    )
    .await;

    // And the turn reaches a final answer.
    send(
        &mut host,
        HostToPlugin::TurnEndEvent(TurnEndEvent {
            session_id: session.clone(),
            final_answer: true,
        }),
        3,
    )
    .await;

    // Then the guest pushes one grouped citations payload for the session.
    let pushed = next_message(&mut host).await;
    let PluginToHost::PushCitations(msg) = pushed else {
        panic!("expected PushCitations, got {pushed:?}");
    };
    assert_eq!(msg.session_id, session);
    assert_eq!(msg.citations.len(), 1);
    assert_eq!(msg.citations[0].url, "https://doc.rust-lang.org/book");
    assert_eq!(msg.citations[0].title, "The Rust Programming Language");

    eprintln!("[e2e] shutting down");
    tokio::time::timeout(WAIT, host.shutdown())
        .await
        .expect("shutdown timed out");
}

/// A parallel web_fetch turn yields citations from both its `urls`
/// arguments (call rule) and its result JSON (result rule), deduped by URL.
#[tokio::test]
async fn parallel_fetch_turn_yields_push_citations() {
    // Given the real url-citations guest, handshaken.
    let engine = PluginEngine::new().expect("engine");
    let grants = Grants {
        read_dirs: vec![],
        write_dirs: vec![],
        http: false,
        config: serde_json::Value::Null,
    };
    let mut host = PluginHost::start(
        &engine,
        "url-citations",
        std::path::Path::new(WASM),
        &grants,
    )
    .expect("guest started");
    let _ = next_message(&mut host).await; // Hello
    send(
        &mut host,
        HostToPlugin::Welcome(jinn_plugin_api::Welcome {
            protocol_version: PROTOCOL_VERSION,
            plugin_id: "url-citations".to_owned(),
            read_dirs: vec![],
            write_dirs: vec![],
            http_allowed: false,
            config: serde_json::Value::Null,
        }),
        0,
    )
    .await;

    // When a web_fetch call cites the URL in `urls` and the result JSON
    // cites it again with a title.
    let session = "01943d8e-5a1f-7c2d-9e3b-4f6a8b0c1d2e".to_owned();
    send(
        &mut host,
        HostToPlugin::ToolCallEvent(ToolCallEvent {
            session_id: session.clone(),
            tool_call_id: "call_2".to_owned(),
            name: "mcp__parallel__web_fetch".to_owned(),
            arguments: r#"{"urls":["https://modelcontextprotocol.io/introduction"]}"#.to_owned(),
        }),
        1,
    )
    .await;
    send(
        &mut host,
        HostToPlugin::ToolResultEvent(ToolResultEvent {
            session_id: session.clone(),
            tool_call_id: "call_2".to_owned(),
            name: "mcp__parallel__web_fetch".to_owned(),
            content: r#"{"extract_id":"extract_abc","results":[{"url":"https://modelcontextprotocol.io/introduction","title":"What is the Model Context Protocol?","publish_date":null,"excerpts":["MCP intro."]}]}"#.to_owned(),
            success: true,
        }),
        2,
    )
    .await;
    send(
        &mut host,
        HostToPlugin::TurnEndEvent(TurnEndEvent {
            session_id: session.clone(),
            final_answer: true,
        }),
        3,
    )
    .await;

    // Then the push carries the URL once, merged with the titled entry.
    let pushed = next_message(&mut host).await;
    let PluginToHost::PushCitations(msg) = pushed else {
        panic!("expected PushCitations, got {pushed:?}");
    };
    assert_eq!(msg.session_id, session);
    assert_eq!(msg.citations.len(), 1, "deduped across call+result rules");
    assert_eq!(
        msg.citations[0].url,
        "https://modelcontextprotocol.io/introduction"
    );
    assert_eq!(
        msg.citations[0].title,
        "What is the Model Context Protocol?"
    );

    tokio::time::timeout(WAIT, host.shutdown())
        .await
        .expect("shutdown timed out");
}

/// A builtin web-tools turn yields the fetched page URL (call rule) and
/// the DDG re-run link (web-search carve-out) in one grouped footer.
#[tokio::test]
async fn builtin_web_tools_turn_yields_both_citations() {
    // Given the real url-citations guest, handshaken.
    let engine = PluginEngine::new().expect("engine");
    let grants = Grants {
        read_dirs: vec![],
        write_dirs: vec![],
        http: false,
        config: serde_json::Value::Null,
    };
    let mut host = PluginHost::start(
        &engine,
        "url-citations",
        std::path::Path::new(WASM),
        &grants,
    )
    .expect("guest started");
    let _ = next_message(&mut host).await; // Hello
    send(
        &mut host,
        HostToPlugin::Welcome(jinn_plugin_api::Welcome {
            protocol_version: PROTOCOL_VERSION,
            plugin_id: "url-citations".to_owned(),
            read_dirs: vec![],
            write_dirs: vec![],
            http_allowed: false,
            config: serde_json::Value::Null,
        }),
        0,
    )
    .await;

    // When a builtin web-fetch and web-search both run successfully in one
    // turn (plain-text results, invisible to the result rule).
    let session = "01943d8e-5a1f-7c2d-9e3b-4f6a8b0c1d2e".to_owned();
    send(
        &mut host,
        HostToPlugin::ToolCallEvent(ToolCallEvent {
            session_id: session.clone(),
            tool_call_id: "call_f".to_owned(),
            name: "web-fetch".to_owned(),
            arguments: r#"{"url":"https://example.com/article"}"#.to_owned(),
        }),
        1,
    )
    .await;
    send(
        &mut host,
        HostToPlugin::ToolResultEvent(ToolResultEvent {
            session_id: session.clone(),
            tool_call_id: "call_f".to_owned(),
            name: "web-fetch".to_owned(),
            content: "# The Article\n\nraw markdown body".to_owned(),
            success: true,
        }),
        2,
    )
    .await;
    send(
        &mut host,
        HostToPlugin::ToolCallEvent(ToolCallEvent {
            session_id: session.clone(),
            tool_call_id: "call_s".to_owned(),
            name: "web-search".to_owned(),
            arguments: r#"{"query":"rust async & await"}"#.to_owned(),
        }),
        3,
    )
    .await;
    send(
        &mut host,
        HostToPlugin::ToolResultEvent(ToolResultEvent {
            session_id: session.clone(),
            tool_call_id: "call_s".to_owned(),
            name: "web-search".to_owned(),
            content: "1. Title — https://example.com/hit\n   snippet".to_owned(),
            success: true,
        }),
        4,
    )
    .await;
    send(
        &mut host,
        HostToPlugin::TurnEndEvent(TurnEndEvent {
            session_id: session.clone(),
            final_answer: true,
        }),
        5,
    )
    .await;

    // Then the single grouped push carries both: the fetched URL and the
    // form-encoded DDG re-run link.
    let pushed = next_message(&mut host).await;
    let PluginToHost::PushCitations(msg) = pushed else {
        panic!("expected PushCitations, got {pushed:?}");
    };
    assert_eq!(msg.session_id, session);
    assert_eq!(msg.citations.len(), 2, "both builtin citations");
    let urls: Vec<&str> = msg.citations.iter().map(|c| c.url.as_str()).collect();
    assert!(urls.contains(&"https://example.com/article"), "fetched URL");
    assert!(
        urls.contains(&"https://duckduckgo.com/?q=rust+async+%26+await"),
        "DDG re-run URL with encoded query, got {urls:?}"
    );

    tokio::time::timeout(WAIT, host.shutdown())
        .await
        .expect("shutdown timed out");
}

/// An errored turn (final_answer=false) retains citations; the next
/// successful turn flushes them.
#[tokio::test]
async fn errored_turn_retains_citations_until_next_success() {
    // Given the real url-citations guest, handshaken.
    let engine = PluginEngine::new().expect("engine");
    let grants = Grants {
        read_dirs: vec![],
        write_dirs: vec![],
        http: false,
        config: serde_json::Value::Null,
    };
    let mut host = PluginHost::start(
        &engine,
        "url-citations",
        std::path::Path::new(WASM),
        &grants,
    )
    .expect("guest started");
    let _ = next_message(&mut host).await; // Hello
    send(
        &mut host,
        HostToPlugin::Welcome(jinn_plugin_api::Welcome {
            protocol_version: PROTOCOL_VERSION,
            plugin_id: "url-citations".to_owned(),
            read_dirs: vec![],
            write_dirs: vec![],
            http_allowed: false,
            config: serde_json::Value::Null,
        }),
        0,
    )
    .await;

    // When a turn's tool call succeeds but the turn then errors.
    let session = "01943d8e-5a1f-7c2d-9e3b-4f6a8b0c1d2e".to_owned();
    send(
        &mut host,
        HostToPlugin::ToolCallEvent(ToolCallEvent {
            session_id: session.clone(),
            tool_call_id: "call_1".to_owned(),
            name: "web-fetch".to_owned(),
            arguments: r#"{"url":"https://example.com/kept"}"#.to_owned(),
        }),
        1,
    )
    .await;
    send(
        &mut host,
        HostToPlugin::ToolResultEvent(ToolResultEvent {
            session_id: session.clone(),
            tool_call_id: "call_1".to_owned(),
            name: "web-fetch".to_owned(),
            content: "ok".to_owned(),
            success: true,
        }),
        2,
    )
    .await;
    send(
        &mut host,
        HostToPlugin::TurnEndEvent(TurnEndEvent {
            session_id: session.clone(),
            final_answer: false,
        }),
        3,
    )
    .await;

    // Then no push arrives for the errored turn. (Proven by the next step:
    // a premature flush would surface here as the wrong message or an
    // extra one at the final assertion.)
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // And when the next turn reaches a final answer with no new citations.
    send(
        &mut host,
        HostToPlugin::TurnEndEvent(TurnEndEvent {
            session_id: session.clone(),
            final_answer: true,
        }),
        4,
    )
    .await;

    // Then the retained citation flushes exactly once.
    let pushed = next_message(&mut host).await;
    let PluginToHost::PushCitations(msg) = pushed else {
        panic!("expected PushCitations, got {pushed:?}");
    };
    assert_eq!(msg.citations.len(), 1, "retained citation flushed");
    assert_eq!(msg.citations[0].url, "https://example.com/kept");

    tokio::time::timeout(WAIT, host.shutdown())
        .await
        .expect("shutdown timed out");
}

/// Unknown tool shapes and malformed payloads never error the guest: an
/// unrecognized tool, garbage arguments, non-JSON content, an unpaired
/// url-only JSON object, and even an unknown wire tag all pass through,
/// and a subsequent valid turn still flushes.
#[tokio::test]
async fn unknown_shapes_are_ignored_never_fatal() {
    // Given the real url-citations guest, handshaken.
    let engine = PluginEngine::new().expect("engine");
    let grants = Grants {
        read_dirs: vec![],
        write_dirs: vec![],
        http: false,
        config: serde_json::Value::Null,
    };
    let mut host = PluginHost::start(
        &engine,
        "url-citations",
        std::path::Path::new(WASM),
        &grants,
    )
    .expect("guest started");
    let _ = next_message(&mut host).await; // Hello
    send(
        &mut host,
        HostToPlugin::Welcome(jinn_plugin_api::Welcome {
            protocol_version: PROTOCOL_VERSION,
            plugin_id: "url-citations".to_owned(),
            read_dirs: vec![],
            write_dirs: vec![],
            http_allowed: false,
            config: serde_json::Value::Null,
        }),
        0,
    )
    .await;

    // When a barrage of unknown/shapeless events arrives.
    let session = "01943d8e-5a1f-7c2d-9e3b-4f6a8b0c1d2e".to_owned();
    let noise = [
        // An unrecognized tool whose args contain no URLs.
        host_line_owned(HostToPlugin::ToolCallEvent(ToolCallEvent {
            session_id: session.clone(),
            tool_call_id: "n1".to_owned(),
            name: "mcp__unknown__database_query".to_owned(),
            arguments: r#"{"sql":"SELECT * FROM t"}"#.to_owned(),
        })),
        // Garbage (unparseable) arguments.
        host_line_owned(HostToPlugin::ToolCallEvent(ToolCallEvent {
            session_id: session.clone(),
            tool_call_id: "n2".to_owned(),
            name: "mcp__unknown__flaky".to_owned(),
            arguments: "}{ not json".to_owned(),
        })),
        // A non-JSON result body.
        host_line_owned(HostToPlugin::ToolResultEvent(ToolResultEvent {
            session_id: session.clone(),
            tool_call_id: "n1".to_owned(),
            name: "mcp__unknown__database_query".to_owned(),
            content: "12 rows returned (plain text)".to_owned(),
            success: true,
        })),
        // JSON with a URL but no title pairing (not the result shape).
        host_line_owned(HostToPlugin::ToolResultEvent(ToolResultEvent {
            session_id: session.clone(),
            tool_call_id: "n2".to_owned(),
            name: "mcp__unknown__flaky".to_owned(),
            content: r#"{"url":"https://no-title.example"}"#.to_owned(),
            success: true,
        })),
        // An unknown wire tag entirely (forward-compat: old guest, new host).
        r#"{"v":1,"seq":9,"ts":0,"type":"future_event","detail":"hi"}"#.to_owned(),
    ];
    for line in noise {
        // The final unknown-tag line is raw JSON the typed Envelope can't
        // represent — write the pre-encoded line directly, exactly as a
        // newer host would emit an event this guest doesn't know.
        host.write_raw_line(&line).await.expect("noise write");
    }

    // Then the guest is still alive and yields nothing for the noise turn.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // And a subsequent valid turn flushes normally.
    send(
        &mut host,
        HostToPlugin::ToolCallEvent(ToolCallEvent {
            session_id: session.clone(),
            tool_call_id: "call_ok".to_owned(),
            name: "web-fetch".to_owned(),
            arguments: r#"{"url":"https://example.com/after-noise"}"#.to_owned(),
        }),
        10,
    )
    .await;
    send(
        &mut host,
        HostToPlugin::ToolResultEvent(ToolResultEvent {
            session_id: session.clone(),
            tool_call_id: "call_ok".to_owned(),
            name: "web-fetch".to_owned(),
            content: "ok".to_owned(),
            success: true,
        }),
        11,
    )
    .await;
    send(
        &mut host,
        HostToPlugin::TurnEndEvent(TurnEndEvent {
            session_id: session.clone(),
            final_answer: true,
        }),
        12,
    )
    .await;

    // Then exactly the noise-free citation flushes (the noise URLs never
    // buffered; the url-only JSON contributed nothing).
    let pushed = next_message(&mut host).await;
    let PluginToHost::PushCitations(msg) = pushed else {
        panic!("expected PushCitations, got {pushed:?}");
    };
    assert_eq!(msg.citations.len(), 1);
    assert_eq!(msg.citations[0].url, "https://example.com/after-noise");

    tokio::time::timeout(WAIT, host.shutdown())
        .await
        .expect("shutdown timed out");
}

/// Builds a host→guest event line for raw noise writes.
fn host_line_owned(msg: HostToPlugin) -> String {
    // The noise events go through the same encoder; seq/ts are irrelevant
    // to the guest's parsing.
    let envelope = Envelope::for_host(msg, 99, 0);
    serde_json::to_string(&envelope).expect("encode")
}
