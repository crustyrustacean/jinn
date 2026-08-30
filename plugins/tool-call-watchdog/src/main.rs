//! The first-party tool-call-watchdog plugin.
//!
//! Subscribes to `tool_result` / `turn_end` host events and watches each
//! session for a tool-failure spiral (see [`watchdog`]): when a session's
//! failure accumulator reaches the configured maximum, the plugin pushes a
//! mirrored `InsertSystemEntry` explaining the kill followed by a mirrored
//! `CancelStream`, and resets the accumulator. A turn that ends in a
//! genuine final answer resets the count; an aborted turn retains it.
//!
//! Wire behavior: `Hello` (with subscriptions) → (await `Welcome`,
//! configure the maximum from its config) → event loop until stdin closes.

mod watchdog;

use std::io::BufRead as _;

use jinn_plugin_api::{Envelope, HostToPlugin, PluginToHostOrHostToPlugin};
use jinn_plugin_sdk::{PluginOutput, hello_with_subscriptions, push, welcome};

fn main() {
    let mut out = PluginOutput::stdout();
    if hello_with_subscriptions(&mut out, "tool-call-watchdog", &["tool_result", "turn_end"])
        .is_err()
    {
        return;
    }
    let Ok(handshake) = welcome() else {
        return;
    };
    let mut state = watchdog::Watchdog::from_welcome(&handshake);

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
        match event {
            HostToPlugin::ToolResultEvent(e) => {
                for message in state.on_tool_result(&e) {
                    let _ = push(&mut out, message);
                }
            }
            HostToPlugin::TurnEndEvent(e) => state.on_turn_end(&e),
            HostToPlugin::Welcome(_) | HostToPlugin::ToolCallEvent(_) => {}
            // Not subscribed to the stream-lifecycle/tick kinds — never delivered.
            HostToPlugin::StreamStartEvent(_)
            | HostToPlugin::StreamEventPing(_)
            | HostToPlugin::StreamEndEvent(_)
            | HostToPlugin::Tick(_) => {}
        }
    }
}
