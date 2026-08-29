//! E2E: the real tool-call-watchdog wasm artifact driven through its wire.
//!
//! Spawns the committed `res/plugins/tool-call-watchdog.wasm` as a real
//! wasmtime guest (no fake), completes the handshake, plays tool-failure
//! sequences through the guest's stdin, and asserts the mirrored
//! `insert_system_entry` + `cancel_stream` lines it writes back in order.
//! This is the acceptance test for "4 tool failures within the recent
//! window cancel the session's stream with an explanation" — everything
//! above the wire (coordinator translation, bus routing) is covered by the
//! plugin-coordinator's fake-guest tests.
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

use jinn_plugin::{Grants, PluginEngine, PluginHost, PluginReader};

use jinn_plugin_api::{
    Envelope, HostToPlugin, PROTOCOL_VERSION, PluginToHost, PluginToHostOrHostToPlugin,
    ToolResultEvent, TurnEndEvent,
};

/// Where the committed artifact lives, relative to this crate.
const WASM: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../res/plugins/tool-call-watchdog.wasm"
);

/// Overall per-step timeout.
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

/// Asserts no guest push arrives within `quiet` — the bounded negative
/// check for "the watchdog did not trip".
///
/// Uses the split [`PluginReader`] (not `PluginHost::read`, whose
/// take-and-restore of the read half is not cancellation-safe): a timeout
/// drop of the split reader is harmless, while dropping an in-flight
/// `PluginHost::read` would permanently corrupt the host. The reader
/// carries the guest's stdout for the rest of the test, so later steps
/// route through the split reader too (see [`read_via_split`]).
async fn assert_no_push(host: &mut PluginHost, quiet: Duration) -> PluginReader {
    let mut reader = host.split();
    match tokio::time::timeout(quiet, reader.read_next()).await {
        Err(_) => {} // quiet window elapsed with no push: the expectation.
        Ok(Ok(Some(envelope))) => panic!("unexpected guest push: {:?}", envelope.msg),
        Ok(Ok(None)) => panic!("guest ended before the quiet window elapsed"),
        Ok(Err(e)) => panic!("guest read failed during quiet window: {e:?}"),
    }
    reader
}

/// Reads one guest push through a split [`PluginReader`].
async fn read_via_split(reader: &mut PluginReader) -> PluginToHost {
    let envelope = tokio::time::timeout(WAIT, reader.read_next())
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

/// A failing tool-result event factory.
fn failure(session: &str, call: &str) -> ToolResultEvent {
    ToolResultEvent {
        session_id: session.to_owned(),
        tool_call_id: call.to_owned(),
        name: "web-fetch".to_owned(),
        content: "boom".to_owned(),
        success: false,
    }
}

/// A successful tool-result event factory.
fn success(session: &str, call: &str) -> ToolResultEvent {
    ToolResultEvent {
        session_id: session.to_owned(),
        tool_call_id: call.to_owned(),
        name: "web-fetch".to_owned(),
        content: "ok".to_owned(),
        success: true,
    }
}

/// A turn-end event factory.
fn turn_end(session: &str, final_answer: bool) -> TurnEndEvent {
    TurnEndEvent {
        session_id: session.to_owned(),
        final_answer,
    }
}

/// Completes the handshake with the given Welcome config.
async fn handshake(host: &mut PluginHost, config: serde_json::Value) {
    let hello = next_message(host).await;
    let PluginToHost::Hello(hello) = hello else {
        panic!("expected Hello first, got {hello:?}");
    };
    assert_eq!(hello.protocol_version, PROTOCOL_VERSION, "protocol version");
    assert_eq!(
        hello.subscriptions,
        vec!["tool_result", "turn_end"],
        "watchdog must subscribe to tool_result and turn_end"
    );
    eprintln!("[e2e] handshake ok; sending Welcome");
    send(
        host,
        HostToPlugin::Welcome(jinn_plugin_api::Welcome {
            protocol_version: PROTOCOL_VERSION,
            plugin_id: "tool-call-watchdog".to_owned(),
            read_dirs: vec![],
            write_dirs: vec![],
            http_allowed: false,
            config,
        }),
        0,
    )
    .await;
}

/// Spawns the real guest.
fn start_guest() -> PluginHost {
    let engine = PluginEngine::new().expect("engine");
    let grants = Grants {
        read_dirs: vec![],
        write_dirs: vec![],
        http: false,
        config: serde_json::Value::Null,
    };
    PluginHost::start(
        &engine,
        "tool-call-watchdog",
        std::path::Path::new(WASM),
        &grants,
    )
    .expect("guest started")
}

/// Four consecutive failing tool results push the watchdog pair: the
/// system entry first, then the cancel — both for the failing session.
// > 10s workspace default: spawns a real wasmtime engine + guest; under
// load (CI contention) the handshake+turn sequence can exceed 10s.
#[rstest::rstest]
#[timeout(std::time::Duration::from_secs(30))]
#[tokio::test]
async fn four_failing_tool_results_send_entry_then_cancel() {
    // Given the real watchdog guest, handshaken with default config.
    let mut host = start_guest();
    handshake(&mut host, serde_json::Value::Null).await;

    // When four tool results fail in a row for one session.
    let session = "01943d8e-5a1f-7c2d-9e3b-4f6a8b0c1d2e".to_owned();
    for call in 1..=4 {
        send(
            &mut host,
            HostToPlugin::ToolResultEvent(failure(&session, &format!("call_{call}"))),
            call,
        )
        .await;
    }

    // Then the guest pushes exactly two messages, in order: the system
    // entry naming the watchdog and the maximum, then the cancel.
    let pushed = next_message(&mut host).await;
    let PluginToHost::InsertSystemEntry(entry) = pushed else {
        panic!("expected InsertSystemEntry first, got {pushed:?}");
    };
    assert_eq!(entry.session_id, session);
    assert!(
        entry.text.contains("tool-call-watchdog") && entry.text.contains('4'),
        "entry must explain the kill: {:?}",
        entry.text
    );

    let pushed = next_message(&mut host).await;
    let PluginToHost::CancelStream(cancel) = pushed else {
        panic!("expected CancelStream second, got {pushed:?}");
    };
    assert_eq!(cancel.session_id, session);

    eprintln!("[e2e] shutting down");
    tokio::time::timeout(WAIT, host.shutdown())
        .await
        .expect("shutdown timed out");
}

/// Successes debit the count and a clean turn end resets it: no trip
/// through the interleaved sequence, and the guest still trips on a later
/// full run of failures.
// > 10s workspace default: spawns a real wasmtime engine + guest; under
// load (CI contention) the handshake+turn sequence can exceed 10s.
#[rstest::rstest]
#[timeout(std::time::Duration::from_secs(30))]
#[tokio::test]
async fn successes_debit_and_clean_turn_end_resets() {
    // Given the real watchdog guest, handshaken.
    let mut host = start_guest();
    handshake(&mut host, serde_json::Value::Null).await;
    let session = "01943d8e-5a1f-7c2d-9e3b-4f6a8b0c1d2e".to_owned();

    // When three failures, one success (debit to 2), one failure (3), and
    // a clean turn end (reset to 0) flow through.
    let mut seq = 0;
    for call in ["f1", "f2", "f3"] {
        seq += 1;
        send(
            &mut host,
            HostToPlugin::ToolResultEvent(failure(&session, call)),
            seq,
        )
        .await;
    }
    seq += 1;
    send(
        &mut host,
        HostToPlugin::ToolResultEvent(success(&session, "s1")),
        seq,
    )
    .await;
    seq += 1;
    send(
        &mut host,
        HostToPlugin::ToolResultEvent(failure(&session, "f4")),
        seq,
    )
    .await;
    seq += 1;
    send(
        &mut host,
        HostToPlugin::TurnEndEvent(turn_end(&session, true)),
        seq,
    )
    .await;

    // Then the watchdog did not trip (no push within the quiet window);
    // the split reader now carries the guest's stdout.
    let mut reader = assert_no_push(&mut host, Duration::from_millis(300)).await;

    // And a later full run of four failures trips normally: the count was
    // reset, not merely stalled (a non-reset counter would have tripped
    // during the interleaved sequence above).
    for call in ["f5", "f6", "f7", "f8"] {
        seq += 1;
        send(
            &mut host,
            HostToPlugin::ToolResultEvent(failure(&session, call)),
            seq,
        )
        .await;
    }
    let pushed = read_via_split(&mut reader).await;
    assert!(matches!(pushed, PluginToHost::InsertSystemEntry(_)));
    let pushed = read_via_split(&mut reader).await;
    assert!(matches!(pushed, PluginToHost::CancelStream(_)));

    tokio::time::timeout(WAIT, host.shutdown())
        .await
        .expect("shutdown timed out");
}

/// A turn aborted mid-way (final_answer=false) retains the count: the
/// retry turn's failures complete the spiral.
// > 10s workspace default: spawns a real wasmtime engine + guest; under
// load (CI contention) the handshake+turn sequence can exceed 10s.
#[rstest::rstest]
#[timeout(std::time::Duration::from_secs(30))]
#[tokio::test]
async fn aborted_turn_retains_the_count() {
    // Given the real watchdog guest, handshaken.
    let mut host = start_guest();
    handshake(&mut host, serde_json::Value::Null).await;
    let session = "01943d8e-5a1f-7c2d-9e3b-4f6a8b0c1d2e".to_owned();

    // When three failures land, the turn aborts, and the retry turn adds
    // one more failure.
    for (seq, call) in ["f1", "f2", "f3"].iter().enumerate() {
        send(
            &mut host,
            HostToPlugin::ToolResultEvent(failure(&session, call)),
            seq as u64 + 1,
        )
        .await;
    }
    send(
        &mut host,
        HostToPlugin::TurnEndEvent(turn_end(&session, false)),
        10,
    )
    .await;
    send(
        &mut host,
        HostToPlugin::ToolResultEvent(failure(&session, "f4")),
        11,
    )
    .await;

    // Then the watchdog trips on the fourth failure overall — the aborted
    // turn did not reset the count.
    let pushed = next_message(&mut host).await;
    assert!(matches!(pushed, PluginToHost::InsertSystemEntry(_)));
    let pushed = next_message(&mut host).await;
    assert!(matches!(pushed, PluginToHost::CancelStream(_)));

    tokio::time::timeout(WAIT, host.shutdown())
        .await
        .expect("shutdown timed out");
}

/// `max_failures` from the Welcome config lowers the trip point: two
/// failures trip a watchdog configured with max_failures = 2.
// > 10s workspace default: spawns a real wasmtime engine + guest; under
// load (CI contention) the handshake+turn sequence can exceed 10s.
#[rstest::rstest]
#[timeout(std::time::Duration::from_secs(30))]
#[tokio::test]
async fn max_failures_config_trips_early() {
    // Given the real watchdog guest, handshaken with max_failures = 2.
    let mut host = start_guest();
    handshake(&mut host, serde_json::json!({ "max_failures": 2 })).await;
    let session = "01943d8e-5a1f-7c2d-9e3b-4f6a8b0c1d2e".to_owned();

    // When two failures arrive.
    send(
        &mut host,
        HostToPlugin::ToolResultEvent(failure(&session, "f1")),
        1,
    )
    .await;
    send(
        &mut host,
        HostToPlugin::ToolResultEvent(failure(&session, "f2")),
        2,
    )
    .await;

    // Then the watchdog trips at two (the default of 4 would not have).
    let pushed = next_message(&mut host).await;
    let PluginToHost::InsertSystemEntry(entry) = pushed else {
        panic!("expected InsertSystemEntry, got {pushed:?}");
    };
    assert!(entry.text.contains('2'), "entry names the configured max");
    let pushed = next_message(&mut host).await;
    assert!(matches!(pushed, PluginToHost::CancelStream(_)));

    tokio::time::timeout(WAIT, host.shutdown())
        .await
        .expect("shutdown timed out");
}

/// A trip latches: the next lone failure after a trip pushes nothing
/// (the accumulator was zeroed), so the session is not re-killed.
// > 10s workspace default: spawns a real wasmtime engine + guest; under
// load (CI contention) the handshake+turn sequence can exceed 10s.
#[rstest::rstest]
#[timeout(std::time::Duration::from_secs(30))]
#[tokio::test]
async fn trip_latches_until_new_failures_accumulate() {
    // Given the real watchdog guest, handshaken, driven to a trip.
    let mut host = start_guest();
    handshake(&mut host, serde_json::Value::Null).await;
    let session = "01943d8e-5a1f-7c2d-9e3b-4f6a8b0c1d2e".to_owned();
    for call in ["f1", "f2", "f3", "f4"] {
        send(
            &mut host,
            HostToPlugin::ToolResultEvent(failure(&session, call)),
            1,
        )
        .await;
    }
    let _ = next_message(&mut host).await; // InsertSystemEntry
    let _ = next_message(&mut host).await; // CancelStream

    // When one more failure arrives.
    send(
        &mut host,
        HostToPlugin::ToolResultEvent(failure(&session, "f5")),
        9,
    )
    .await;

    // Then nothing is pushed — the latch reset the accumulator to 0.
    assert_no_push(&mut host, Duration::from_millis(300)).await;
    tokio::time::timeout(WAIT, host.shutdown())
        .await
        .expect("shutdown timed out");
}
