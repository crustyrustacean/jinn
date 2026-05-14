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
    host_status_bar_add_slot("turn-counter", "left", 50, "Turn: 0");
}

fn on_event(event) {
    // Always recompute from the active session's history.
    let entries = host_chat_get_entries();
    let count = compute_turn_count(entries);
    host_status_bar_update_slot("turn-counter", "Turn: " + count);
}

fn compute_turn_count(entries) {
    let c = 0;
    for e in entries {
        if e.kind == "user" || e.kind == "assistant" {
            c += 1;
        }
    }
    c
}
