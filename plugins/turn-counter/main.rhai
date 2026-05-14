// Turn counter plugin — counts conversation turns in the status bar.
//
// A "turn" is when a participant gives up control:
// - User sends a message -> 1 turn
// - Model finishes a response (not tool-use) -> 1 turn

let count = 0;
let slot_id = "";

fn init() {
    // Subscribe to events that mark turn boundaries.
    host_events_subscribe([
        "chat_input::ChatEntrySubmitted",
        "provider::StreamCompleted",
    ]);

    // Register a status bar slot.
    slot_id = host_status_bar_add_slot("turn-counter", "left", 50, "Turn: 0");

    // Recompute from chat history (handles reload).
    let entries = host_chat_get_entries();
    count = compute_turn_count(entries);
    host_status_bar_update_slot(slot_id, "Turn: " + count);
}

fn on_event(event) {
    if event.type == "chat_input::ChatEntrySubmitted" && event.kind == "user" {
        count += 1;
    } else if event.type == "provider::StreamCompleted" && event.reason == "finished" {
        count += 1;
    }
    host_status_bar_update_slot(slot_id, "Turn: " + count);
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
