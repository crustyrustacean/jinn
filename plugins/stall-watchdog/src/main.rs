//! The first-party stall-watchdog plugin.
//!
//! Subscribes to `stream_start` / `stream_event` / `stream_end` / `tick`
//! host events and detects a silent LLM stream (see [`watchdog`]): a session
//! whose in-flight provider stream produces nothing for `timeout_secs`
//! restarts (mirrored `RestartStalledStream`), up to `max_restarts` times
//! before the plugin gives up with a system entry plus a mirrored
//! `CancelStream`.
//!
//! Tool execution is deliberately invisible to this plugin: a stream ended
//! with `ToolUse` is disarmed, so a quiet tool batch can never trip the
//! watchdog — tool hangs are their own domain's problem.
//!
//! Wire behavior: `Hello` (with subscriptions) → (await `Welcome`, configure
//! the timeout/budget from its config) → event loop until stdin closes.

mod watchdog;

use std::io::BufRead as _;

use jinn_plugin_api::{Envelope, HostToPlugin, PluginToHostOrHostToPlugin};
use jinn_plugin_sdk::{PluginOutput, hello_with_subscriptions, push, welcome};

fn main() {
    let mut out = PluginOutput::stdout();
    if hello_with_subscriptions(
        &mut out,
        "stall-watchdog",
        &["stream_start", "stream_event", "stream_end", "tick"],
    )
    .is_err()
    {
        return;
    }
    let Ok(handshake) = welcome() else {
        return;
    };
    let mut state = watchdog::StallWatchdog::from_welcome(&handshake);

    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    while let Some(Ok(line)) = lines.next() {
        let Ok(envelope) = serde_json::from_str::<Envelope>(&line) else {
            continue;
        };
        let event = match envelope.msg {
            PluginToHostOrHostToPlugin::Host(event) => event,
            _ => continue,
        };
        // The guest clock: events carry no timestamp, so arrival time is
        // taken locally and kept in the same epoch-millisecond unit the
        // host's tick pulses use.
        let now = now_ms();
        match event {
            HostToPlugin::StreamStartEvent(e) => state.on_stream_start(&e, now),
            HostToPlugin::StreamEventPing(e) => state.on_stream_event(&e, now),
            HostToPlugin::StreamEndEvent(e) => state.on_stream_end(&e),
            HostToPlugin::Tick(t) => {
                for message in state.on_tick(t.now_ms) {
                    let _ = push(&mut out, message);
                }
            }
            HostToPlugin::Welcome(_)
            | HostToPlugin::ToolCallEvent(_)
            | HostToPlugin::ToolResultEvent(_)
            | HostToPlugin::TurnEndEvent(_) => {}
        }
    }
}

/// Unix epoch milliseconds — the guest's single clock source for stall math.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
