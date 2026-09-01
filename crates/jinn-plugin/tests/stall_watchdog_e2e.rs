//! E2E: the real stall-watchdog wasm artifact driven through its wire.
//!
//! Spawns the committed `res/plugins/stall-watchdog.wasm` as a real wasmtime
//! guest (no fake), completes the handshake, and plays stream-start / ping /
//! stream-end / tick sequences through the guest's stdin, asserting the
//! `insert_system_entry` + `restart_stalled_stream` pairs (and the final
//! give-up `insert_system_entry` + `cancel_stream` pair) arrive in order.
//! Silence windows use the configured `timeout_secs` against the same real
//! clock the guest stamps events with, so each window costs one short sleep.
//! Everything above the wire (coordinator translation, bus routing, and the
//! session actor's restart handling) is covered by the plugin-coordinator
//! fake-guest tests and the session actor's stall-retry tests.
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
    StreamEndEvent, StreamEndReason, StreamEventPing, StreamStartEvent, TickEvent,
};

/// Where the committed artifact lives, relative to this crate.
const WASM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../res/plugins/stall-watchdog.wasm");

/// Overall per-step timeout.
const WAIT: Duration = Duration::from_secs(10);

/// The silence window the tests configure (`timeout_secs: 2`), as a sleep
/// budget per window — the guest stamps real arrival times, so the host must
/// actually let the window elapse before the trip tick.
const WINDOW: Duration = Duration::from_secs(2);

/// The test's config: a short window and a two-restart budget, so a full
/// restart→give-up lineage fits inside the per-test timeout.
fn watchdog_config() -> serde_json::Value {
    serde_json::json!({ "timeout_secs": WINDOW.as_secs(), "max_restarts": 2 })
}

/// Unix epoch milliseconds — the same clock the guest stamps events with.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

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

/// Event factories.
fn start_event(session: &str) -> StreamStartEvent {
    StreamStartEvent {
        session_id: session.to_owned(),
    }
}

fn ping(session: &str) -> StreamEventPing {
    StreamEventPing {
        session_id: session.to_owned(),
    }
}

fn end_event(session: &str, reason: StreamEndReason) -> StreamEndEvent {
    StreamEndEvent {
        session_id: session.to_owned(),
        reason,
    }
}

fn tick(now_ms: u64) -> TickEvent {
    TickEvent { now_ms }
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
        vec![
            "stream_start".to_owned(),
            "stream_event".to_owned(),
            "stream_end".to_owned(),
            "tick".to_owned(),
        ],
        "watchdog must subscribe to the stream lifecycle and tick events"
    );
    eprintln!("[e2e] handshake ok; sending Welcome");
    send(
        host,
        HostToPlugin::Welcome(jinn_plugin_api::Welcome {
            protocol_version: PROTOCOL_VERSION,
            plugin_id: "stall-watchdog".to_owned(),
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
        "stall-watchdog",
        std::path::Path::new(WASM),
        &grants,
    )
    .expect("guest started")
}

/// Arms the timer by sending `stream_start` at the real clock's `now`, and
/// returns that epoch-ms base for later tick math.
async fn arm_at_now(host: &mut PluginHost, session: &str, seq: u64) -> u64 {
    let base = now_ms();
    send(
        host,
        HostToPlugin::StreamStartEvent(start_event(session)),
        seq,
    )
    .await;
    base
}

/// Runs one silence window: sleeps past the window, then sends a tick at the
/// real clock's `now`.
async fn elapse_window_and_tick(host: &mut PluginHost, seq: u64) {
    tokio::time::sleep(WINDOW + Duration::from_millis(150)).await;
    send(host, HostToPlugin::Tick(tick(now_ms())), seq).await;
}

/// Two consecutive silent windows push the restart pair for each attempt, in
/// order: marker entry then restart request, attempt 1 then attempt 2.
// > 10s workspace default: two real 2s silence windows plus wasmtime guest
// startup; under load (CI contention) the sequence can exceed 10s.
#[rstest::rstest]
#[timeout(std::time::Duration::from_secs(30))]
#[tokio::test]
async fn consecutive_silent_windows_push_ordered_restart_pairs() {
    // Given the real watchdog guest, handshaken with a 2s window and a
    // restart budget of 2.
    let mut host = start_guest();
    handshake(&mut host, watchdog_config()).await;
    let session = "01943d8e-5a1f-7c2d-9e3b-4f6a8b0c1d2e".to_owned();

    // When the stream starts and then goes silent for a full window, twice.
    let _base = arm_at_now(&mut host, &session, 1).await;

    elapse_window_and_tick(&mut host, 2).await;
    let pushed = next_message(&mut host).await;
    let PluginToHost::InsertSystemEntry(entry) = pushed else {
        panic!("expected the attempt-1 marker entry first, got {pushed:?}");
    };
    assert_eq!(entry.session_id, session);
    assert!(
        entry.text.contains("attempt 1 of 2"),
        "marker must name attempt 1 of 2, got: {:?}",
        entry.text
    );
    let pushed = next_message(&mut host).await;
    let PluginToHost::RestartStalledStream(restart) = pushed else {
        panic!("expected the attempt-1 restart second, got {pushed:?}");
    };
    assert_eq!(restart.session_id, session);
    assert_eq!(restart.attempt, 1);
    assert_eq!(restart.max_restarts, 2);

    elapse_window_and_tick(&mut host, 3).await;
    let pushed = next_message(&mut host).await;
    let PluginToHost::InsertSystemEntry(entry) = pushed else {
        panic!("expected the attempt-2 marker entry first, got {pushed:?}");
    };
    assert!(
        entry.text.contains("attempt 2 of 2"),
        "marker must name attempt 2 of 2, got: {:?}",
        entry.text
    );
    let pushed = next_message(&mut host).await;
    let PluginToHost::RestartStalledStream(restart) = pushed else {
        panic!("expected the attempt-2 restart second, got {pushed:?}");
    };
    assert_eq!(restart.attempt, 2);

    tokio::time::timeout(WAIT, host.shutdown())
        .await
        .expect("shutdown timed out");
}

/// A third silent window exhausts the budget: the guest pushes the give-up
/// pair (entry then cancel) and pushes nothing further.
// > 10s workspace default: three real 2s silence windows plus wasmtime
// guest startup; under load (CI contention) the sequence can exceed 10s.
#[rstest::rstest]
#[timeout(std::time::Duration::from_secs(40))]
#[tokio::test]
async fn budget_exhaustion_gives_up_with_entry_then_cancel_then_stays_silent() {
    // Given the real watchdog guest, handshaken with a 2s window and a
    // restart budget of 2.
    let mut host = start_guest();
    handshake(&mut host, watchdog_config()).await;
    let session = "01943d8e-5a1f-7c2d-9e3b-4f6a8b0c1d2e".to_owned();
    let _base = arm_at_now(&mut host, &session, 1).await;

    // When two silent windows restart within budget, and a third stays
    // silent past the window.
    elapse_window_and_tick(&mut host, 2).await;
    let _ = next_message(&mut host).await; // attempt-1 marker
    let _ = next_message(&mut host).await; // attempt-1 restart
    elapse_window_and_tick(&mut host, 3).await;
    let _ = next_message(&mut host).await; // attempt-2 marker
    let _ = next_message(&mut host).await; // attempt-2 restart
    elapse_window_and_tick(&mut host, 4).await;

    // Then the guest surrenders: the give-up entry first, then the cancel.
    let pushed = next_message(&mut host).await;
    let PluginToHost::InsertSystemEntry(entry) = pushed else {
        panic!("expected the give-up entry first, got {pushed:?}");
    };
    assert_eq!(entry.session_id, session);
    assert!(
        entry.text.contains("giving up"),
        "surrender must be explained, got: {:?}",
        entry.text
    );
    let pushed = next_message(&mut host).await;
    let PluginToHost::CancelStream(cancel) = pushed else {
        panic!("expected the cancel second, got {pushed:?}");
    };
    assert_eq!(cancel.session_id, session);

    // And a further tick pushes nothing — the give-up disarmed the timer.
    send(&mut host, HostToPlugin::Tick(tick(now_ms())), 5).await;
    assert_no_push(&mut host, Duration::from_millis(300)).await;

    tokio::time::timeout(WAIT, host.shutdown())
        .await
        .expect("shutdown timed out");
}

/// A `stream_event` ping inside the window resets it: an early tick pushes
/// nothing, and the timer still trips a full window after the ping (it was
/// reset, not disarmed).
// > 10s workspace default: two real ~1.2s sleeps plus wasmtime guest
// startup; under load (CI contention) the sequence can exceed 10s.
#[rstest::rstest]
#[timeout(std::time::Duration::from_secs(30))]
#[tokio::test]
async fn stream_ping_resets_the_silence_window() {
    // Given the real watchdog guest, handshaken, timer armed by stream_start.
    let mut host = start_guest();
    handshake(&mut host, watchdog_config()).await;
    let session = "01943d8e-5a1f-7c2d-9e3b-4f6a8b0c1d2e".to_owned();
    let _base = arm_at_now(&mut host, &session, 1).await;

    // When the stream pings almost immediately, and a tick arrives before a
    // full window has passed since the ping.
    send(&mut host, HostToPlugin::StreamEventPing(ping(&session)), 2).await;
    tokio::time::sleep(WINDOW / 2).await;
    send(&mut host, HostToPlugin::Tick(tick(now_ms())), 3).await;

    // Then nothing is pushed — the window restarted from the ping.
    let mut reader = assert_no_push(&mut host, Duration::from_millis(300)).await;

    // And once a full window passes after the ping, the watchdog trips —
    // proving the ping reset (not disarmed) the timer.
    elapse_window_and_tick(&mut host, 4).await;
    let pushed = read_via_split(&mut reader).await;
    let PluginToHost::InsertSystemEntry(entry) = pushed else {
        panic!("expected the marker entry after the reset window, got {pushed:?}");
    };
    assert!(entry.text.contains("attempt 1 of 2"), "got: {:?}", entry.text);
    let pushed = read_via_split(&mut reader).await;
    assert!(matches!(pushed, PluginToHost::RestartStalledStream(_)));

    tokio::time::timeout(WAIT, host.shutdown())
        .await
        .expect("shutdown timed out");
}

/// A stream ending in `Finished` disarms the timer entirely: later ticks
/// push nothing.
#[rstest::rstest]
#[timeout(std::time::Duration::from_secs(30))]
#[tokio::test]
async fn finished_stream_end_disarms_the_timer() {
    // Given the real watchdog guest, handshaken, timer armed by stream_start.
    let mut host = start_guest();
    handshake(&mut host, watchdog_config()).await;
    let session = "01943d8e-5a1f-7c2d-9e3b-4f6a8b0c1d2e".to_owned();
    let _base = arm_at_now(&mut host, &session, 1).await;

    // When the stream ends in a genuine completion, and silence follows.
    send(
        &mut host,
        HostToPlugin::StreamEndEvent(end_event(&session, StreamEndReason::Finished)),
        2,
    )
    .await;
    elapse_window_and_tick(&mut host, 3).await;

    // Then nothing is pushed — a finished stream has no timer to trip.
    assert_no_push(&mut host, Duration::from_millis(300)).await;

    tokio::time::timeout(WAIT, host.shutdown())
        .await
        .expect("shutdown timed out");
}
