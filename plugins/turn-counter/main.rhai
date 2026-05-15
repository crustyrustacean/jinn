// Turn counter plugin — counts conversation turns in the status bar.
//
// A "turn" is when a participant gives up control:
// - User sends a message -> 1 turn
// - Model finishes a response (not tool-use) -> 1 turn
//
// Always recomputes from the current session's chat history so the count
// is correct after session switches, reloads, etc.

fn init() {
    // Subscribe to events that mark turn boundaries.
    host_events_subscribe([
        "chat_input::ChatEntrySubmitted",
        "provider::StreamCompleted",
        "system::ActiveSessionChanged",
    ]);

    // Register a status bar slot.
    host_status_bar_add_slot("turn-counter", "left", 50, "Turns: 0");
}

fn on_event(event) {
    // Always recompute from the active session's history.
    let entries = host_chat_get_entries();
    let count = compute_turn_count(entries);
    host_status_bar_update_slot("turn-counter", "Turns: " + count);
}

fn compute_turn_count(entries) {
    let c = 0;
    let len = entries.len();
    for i in range(0, len) {
        let e = entries[i];
        if e.kind == "user" {
            c += 1;
        } else if e.kind == "assistant" {
            // Only count as a turn if NOT followed by a tool_call.
            // An assistant followed by tool_call is an intermediate
            // step in the tool loop — part of the same turn.
            let is_last = i == len - 1;
            if is_last || entries[i + 1].kind != "tool_call" {
                c += 1;
            }
        }
    }
    c
}
