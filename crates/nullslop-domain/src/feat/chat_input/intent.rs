//! Chat input box intent handlers.
//!
//! Handles 17 chat-input intents:
//!
//! - **InsertChar** — inserts a character, manages autocomplete triggering/filtering/expansion.
//! - **PasteText** — bulk inserts pasted text, deactivates autocomplete.
//! - **DeleteGrapheme** — backspace with autocomplete awareness.
//! - **DeleteGraphemeForward** — forward delete with autocomplete awareness.
//! - **SubmitMessage** — validates, extracts text, resets buffer, returns `EnqueueUserMessage`.
//! - **AutocompleteConfirm** — confirms autocomplete selection or falls back to tab switch.
//! - **Cursor movement** (8 intents) — move cursor, optionally deactivating autocomplete.
//! - **EnterInsertMode** — switches to Input mode.
//! - **EnterNormalMode** — cancels streams, clears picker, switches to Normal mode.
//! - **NormalEscape** — clears chat entry selection.

use crate::common::app_state::AppState;
use crate::feat::chat_input::AutocompleteMatch;
use crate::feat::chat_input::AutocompleteTrigger;
use crate::feat::chat_input::ChatInputBoxState;
use crate::feat::chat_input::protocol::command::EnqueueUserMessage;
use crate::feat::chat_input::slash_command::SlashCommand;
use crate::feat::chat_input::state::autocomplete::AutocompleteState;
use crate::feat::compaction_actor::protocol::command::EnqueueCompaction;
use crate::feat::context::prompt_template::PromptTemplateStore;
use crate::feat::session::chat_session::SessionPhase;
use crate::protocol::{ChatEntry, Command, IntentResult};
use unicode_segmentation::UnicodeSegmentation as _;

use super::validator;

// --- Character input ---

/// Handles `InsertChar` — inserts a character and manages autocomplete.
pub fn handle_insert_char(ch: char, state: &mut AppState) -> IntentResult {
    let is_autocomplete_active = state.active_chat_input().autocomplete().is_some();

    if is_autocomplete_active {
        state.active_chat_input_mut().insert_grapheme_at_cursor(ch);

        let trigger = state
            .active_chat_input()
            .autocomplete()
            .as_ref()
            .map(AutocompleteState::trigger);

        match (ch, trigger) {
            (' ', _) => {
                state.active_chat_input_mut().deactivate_autocomplete();
            }
            ('#', Some(AutocompleteTrigger::Hash)) => {
                let Some(token_start) = state.active_chat_input().autocomplete_token_start() else {
                    return IntentResult::empty();
                };
                let cursor_before_insert = state.active_chat_input().cursor_pos() - 1;
                let filter: String = state
                    .active_chat_input()
                    .text()
                    .graphemes(true)
                    .enumerate()
                    .skip_while(|(i, _)| *i < token_start + 1)
                    .take_while(|(i, _)| *i < cursor_before_insert)
                    .map(|(_, g)| g)
                    .collect();
                if let Some(template) = state.context.prompt_templates.find_by_name(&filter) {
                    let body = template.body.clone();
                    state.active_chat_input_mut().expand_autocomplete(&body);
                } else {
                    state.active_chat_input_mut().deactivate_autocomplete();
                }
            }
            _ => {
                let filter = state
                    .active_chat_input()
                    .autocomplete_filter()
                    .unwrap_or_default();
                let matches =
                    compute_updated_matches(&state.context.prompt_templates, trigger, &filter);
                state
                    .active_chat_input_mut()
                    .update_autocomplete_matches(matches);
            }
        }
    } else {
        state.active_chat_input_mut().insert_grapheme_at_cursor(ch);

        match ch {
            '#' => {
                let input = state.active_chat_input();
                if is_valid_hash_trigger_position(input) {
                    let token_start = input.cursor_pos() - 1;
                    let matches = compute_matches(&state.context.prompt_templates, "");
                    state.active_chat_input_mut().activate_autocomplete(
                        token_start,
                        AutocompleteTrigger::Hash,
                        matches,
                    );
                }
            }
            '/' => {
                let input = state.active_chat_input();
                if is_valid_slash_trigger_position(input) {
                    let token_start = input.cursor_pos() - 1;
                    let matches = compute_slash_matches("");
                    state.active_chat_input_mut().activate_autocomplete(
                        token_start,
                        AutocompleteTrigger::Slash,
                        matches,
                    );
                }
            }
            _ => {}
        }
    }

    IntentResult::empty()
}

// --- Paste ---

/// Handles `PasteText` — bulk inserts pasted text and deactivates autocomplete.
///
/// Pastes bypass the per-character insertion pipeline entirely, inserting the
/// full string in one O(n) operation. Autocomplete is always deactivated on
/// paste since the pasted content may span multiple lines or tokens.
pub fn handle_paste_text(text: &str, state: &mut AppState) -> IntentResult {
    state.active_chat_input_mut().deactivate_autocomplete();
    state.active_chat_input_mut().insert_text(text);
    IntentResult::empty()
}

// --- Deletion ---

/// Handles `DeleteGrapheme` — backspace with autocomplete awareness.
pub fn handle_delete_grapheme(state: &mut AppState) -> IntentResult {
    let should_deactivate =
        if let Some(token_start) = state.active_chat_input().autocomplete_token_start() {
            state.active_chat_input().cursor_pos() <= token_start + 1
        } else {
            false
        };

    if should_deactivate {
        state.active_chat_input_mut().deactivate_autocomplete();
        state
            .active_chat_input_mut()
            .delete_grapheme_before_cursor();
    } else if state.active_chat_input().autocomplete().is_some() {
        state
            .active_chat_input_mut()
            .delete_grapheme_before_cursor();
        let filter = state
            .active_chat_input()
            .autocomplete_filter()
            .unwrap_or_default();
        let trigger = state
            .active_chat_input()
            .autocomplete()
            .as_ref()
            .map(AutocompleteState::trigger);
        let matches = compute_updated_matches(&state.context.prompt_templates, trigger, &filter);
        state
            .active_chat_input_mut()
            .update_autocomplete_matches(matches);
    } else {
        state
            .active_chat_input_mut()
            .delete_grapheme_before_cursor();
    }

    try_reactivate_autocomplete(state);
    IntentResult::empty()
}

/// Handles `DeleteGraphemeForward` — forward delete with autocomplete awareness.
pub fn handle_delete_grapheme_forward(state: &mut AppState) -> IntentResult {
    let token_start = state.active_chat_input().autocomplete_token_start();
    let cursor = state.active_chat_input().cursor_pos();

    if let Some(token_start) = token_start {
        if cursor == token_start {
            state.active_chat_input_mut().deactivate_autocomplete();
            state.active_chat_input_mut().delete_grapheme_after_cursor();
        } else {
            state.active_chat_input_mut().delete_grapheme_after_cursor();
            let should_deactivate = should_deactivate_on_cursor_move(state);
            if should_deactivate {
                state.active_chat_input_mut().deactivate_autocomplete();
            } else {
                let filter = state
                    .active_chat_input()
                    .autocomplete_filter()
                    .unwrap_or_default();
                let trigger = state
                    .active_chat_input()
                    .autocomplete()
                    .as_ref()
                    .map(AutocompleteState::trigger);
                let matches =
                    compute_updated_matches(&state.context.prompt_templates, trigger, &filter);
                state
                    .active_chat_input_mut()
                    .update_autocomplete_matches(matches);
            }
        }
    } else {
        state.active_chat_input_mut().delete_grapheme_after_cursor();
    }

    try_reactivate_autocomplete(state);
    IntentResult::empty()
}

// --- Submission ---

/// Handles `SubmitMessage` — confirms autocomplete if active, executes slash commands,
/// or submits the message as chat input.
pub fn handle_submit_message(state: &mut AppState) -> IntentResult {
    if state.active_chat_input().autocomplete().is_some() {
        return handle_submit_message_with_autocomplete(state);
    }

    if validator::validate_submit_message(state).is_err() {
        return IntentResult::empty();
    }

    let display = state.active_chat_input().text().to_owned();

    // Check for slash command execution.
    if let Some(command_name) = display.strip_prefix('/') {
        // Extract the first word after / (command name, ignoring arguments).
        let cmd = command_name.split_whitespace().next().unwrap_or("");
        if let Some(cmd) = SlashCommand::lookup(cmd) {
            state.active_chat_input_mut().reset();
            return execute_slash_command(cmd, &display, state);
        }
        // Unknown /command — fall through to normal message.
    }

    let session_id = state.session.active_session_id().clone();
    let expanded = crate::feat::context::prompt_template::expand_tokens(
        &display,
        &state.context.prompt_templates,
    );
    state.active_chat_input_mut().reset();

    IntentResult::with_commands(vec![Command::EnqueueUserMessage(EnqueueUserMessage {
        session_id,
        entry: ChatEntry::user_expanded(display, expanded),
    })])
}

/// Handles Enter when autocomplete is active — completes the selection and submits.
///
/// For `Hash` trigger: completes the name into the buffer, then submits as a
/// normal chat message (the completed name is just text in the message).
/// For `Slash` trigger: completes the command name, then re-checks for slash
/// command execution.
fn handle_submit_message_with_autocomplete(state: &mut AppState) -> IntentResult {
    let trigger = state
        .active_chat_input()
        .autocomplete()
        .as_ref()
        .map(AutocompleteState::trigger);

    // Complete the selection.
    if let Some(selected) = state.active_chat_input().autocomplete_selected() {
        let name = selected.name.clone();
        state.active_chat_input_mut().complete_autocomplete(&name);
    }
    state.active_chat_input_mut().deactivate_autocomplete();

    // Now submit based on what we completed.
    if validator::validate_submit_message(state).is_err() {
        return IntentResult::empty();
    }

    let display = state.active_chat_input().text().to_owned();

    match trigger {
        Some(AutocompleteTrigger::Slash) => {
            // Check for slash command execution after completion.
            if let Some(command_name) = display.strip_prefix('/') {
                let cmd = command_name.split_whitespace().next().unwrap_or("");
                if let Some(cmd) = SlashCommand::lookup(cmd) {
                    state.active_chat_input_mut().reset();
                    return execute_slash_command(cmd, &display, state);
                }
            }
            // Fall through to normal submit.
        }
        _ => {}
    }

    let session_id = state.session.active_session_id().clone();
    let expanded = crate::feat::context::prompt_template::expand_tokens(
        &display,
        &state.context.prompt_templates,
    );
    state.active_chat_input_mut().reset();

    IntentResult::with_commands(vec![Command::EnqueueUserMessage(EnqueueUserMessage {
        session_id,
        entry: ChatEntry::user_expanded(display, expanded),
    })])
}

/// Executes a slash command.
fn execute_slash_command(
    command: SlashCommand,
    _display: &str,
    state: &mut AppState,
) -> IntentResult {
    match command {
        SlashCommand::Compact => {
            let session_id = state.session.active_session_id().clone();
            IntentResult::with_commands(vec![Command::EnqueueCompaction(EnqueueCompaction {
                session_id,
            })])
        }
        SlashCommand::New => crate::feat::session::intent::handle_session_new(state),
        SlashCommand::Workflow => {
            let workflow_id = crate::feat::workflow::workflow_state::WorkflowId::new();
            state.frontend.active_tab = crate::protocol::tab::ActiveTab::Workflow;
            state.frontend.scope_stack.swap_base(
                crate::common::app_state::FocusScope::Workflow,
            );
            IntentResult::with_commands(vec![Command::StartWorkflow(
                crate::feat::workflow::protocol::command::StartWorkflow {
                    name: "add-numbers".to_owned(),
                    workflow_id,
                },
            )])
        }
    }
}

// --- Autocomplete ---

/// Handles `AutocompleteConfirm` — confirms selection or falls back to tab switch.
pub fn handle_autocomplete_confirm(state: &mut AppState) -> IntentResult {
    if validator::validate_autocomplete_confirm(state).is_ok() {
        if let Some(selected) = state.active_chat_input().autocomplete_selected() {
            let name = selected.name.clone();
            state.active_chat_input_mut().complete_autocomplete(&name);
            let filter = state
                .active_chat_input()
                .autocomplete_filter()
                .unwrap_or_default();
            let trigger = state
                .active_chat_input()
                .autocomplete()
                .as_ref()
                .map(AutocompleteState::trigger);
            let matches =
                compute_updated_matches(&state.context.prompt_templates, trigger, &filter);
            state
                .active_chat_input_mut()
                .update_autocomplete_matches(matches);
        }
        IntentResult::empty()
    } else {
        // No autocomplete active — no-op.
        IntentResult::empty()
    }
}

// --- Cursor movement ---

/// Handles `MoveCursorLeft` — moves cursor left, deactivates autocomplete if needed.
pub fn handle_move_cursor_left(state: &mut AppState) -> IntentResult {
    state.active_chat_input_mut().move_cursor_left();
    let should_deactivate = should_deactivate_on_cursor_move(state);
    if should_deactivate {
        state.active_chat_input_mut().deactivate_autocomplete();
    }
    try_reactivate_autocomplete(state);
    IntentResult::empty()
}

/// Handles `MoveCursorRight` — moves cursor right, deactivates autocomplete if needed.
pub fn handle_move_cursor_right(state: &mut AppState) -> IntentResult {
    state.active_chat_input_mut().move_cursor_right();
    let should_deactivate = should_deactivate_on_cursor_move(state);
    if should_deactivate {
        state.active_chat_input_mut().deactivate_autocomplete();
    }
    try_reactivate_autocomplete(state);
    IntentResult::empty()
}

/// Handles `MoveCursorToStart` — moves cursor to start, deactivates autocomplete.
pub fn handle_move_cursor_to_start(state: &mut AppState) -> IntentResult {
    state.active_chat_input_mut().deactivate_autocomplete();
    state.active_chat_input_mut().move_cursor_to_start();
    IntentResult::empty()
}

/// Handles `MoveCursorToEnd` — moves cursor to end, deactivates autocomplete.
pub fn handle_move_cursor_to_end(state: &mut AppState) -> IntentResult {
    state.active_chat_input_mut().deactivate_autocomplete();
    state.active_chat_input_mut().move_cursor_to_end();
    IntentResult::empty()
}

/// Handles `MoveCursorWordLeft` — moves cursor one word left, deactivates autocomplete.
pub fn handle_move_cursor_word_left(state: &mut AppState) -> IntentResult {
    state.active_chat_input_mut().deactivate_autocomplete();
    state.active_chat_input_mut().move_cursor_word_left();
    IntentResult::empty()
}

/// Handles `MoveCursorWordRight` — moves cursor one word right, deactivates autocomplete.
pub fn handle_move_cursor_word_right(state: &mut AppState) -> IntentResult {
    state.active_chat_input_mut().deactivate_autocomplete();
    state.active_chat_input_mut().move_cursor_word_right();
    IntentResult::empty()
}

/// Handles `MoveCursorUp` — moves up in autocomplete or moves cursor up.
pub fn handle_move_cursor_up(state: &mut AppState) -> IntentResult {
    if state.active_chat_input().autocomplete().is_some() {
        state.active_chat_input_mut().autocomplete_move_up();
    } else {
        state.active_chat_input_mut().move_cursor_up();
    }
    IntentResult::empty()
}

/// Handles `MoveCursorDown` — moves down in autocomplete or moves cursor down.
pub fn handle_move_cursor_down(state: &mut AppState) -> IntentResult {
    if state.active_chat_input().autocomplete().is_some() {
        state.active_chat_input_mut().autocomplete_move_down();
    } else {
        state.active_chat_input_mut().move_cursor_down();
    }
    IntentResult::empty()
}

// --- Normal Escape ---

/// Handles `NormalEscape` — no-op for selection (always-selected invariant).
///
/// If the session is busy (streaming/sending), activates the cancel stream
/// confirmation prompt. Otherwise, does nothing.
pub fn handle_normal_escape(state: &mut AppState) -> IntentResult {
    super::validator::validate_normal_escape(state);

    if !matches!(state.active_session().phase(), SessionPhase::Idle) {
        // Session is busy — show cancel confirmation prompt.
        state.frontend.cancel_stream_prompt = true;
    }

    IntentResult::empty()
}

// --- Mode transitions ---

/// Handles `EnterInsertMode` — pushes Input onto the scope stack.
pub fn handle_enter_insert_mode(state: &mut AppState) -> IntentResult {
    use crate::common::app_state::FocusScope;

    // If entering from pins, restore the viewport to its pre-pin position.
    // The pin cursor jump is only for pin → Normal, not pin → Insert.
    if state.active_session().has_saved_history_position() {
        state.active_session_mut().restore_history_position();
    }

    state.frontend.scope_stack.push(FocusScope::Input);
    IntentResult::empty()
}

/// Handles `EnterNormalMode` — pops the scope stack (restores previous scope).
///
/// Simply switches out of the current mode. Does NOT cancel streams or drain
/// queues — the cancel confirmation prompt handles that via `NormalEscape`.
pub fn handle_enter_normal_mode(state: &mut AppState) -> IntentResult {
    // If autocomplete is active, dismiss it and stay in the current scope.
    // Two-level ESC: first press closes popup, second press exits mode.
    if state.active_chat_input().autocomplete().is_some() {
        state.active_chat_input_mut().deactivate_autocomplete();
        return IntentResult::empty();
    }

    // If leaving the theme picker without confirming, restore the original theme.
    if state.frontend.scope_stack.picker_kind() == Some(&crate::protocol::PickerKind::Theme)
        && let Some(original) = state.frontend.theme_preview_original.take()
    {
        state.frontend.theme = original;
    }

    // Clear all overlay scopes — always returns to Normal.
    // Using clear_overlays() instead of pop() ensures that ESC from Input mode
    // always lands in Normal, even when a sidebar scope is stacked below Input
    // (e.g., [Normal, SidebarPersona, Input] → [Normal]).
    state.frontend.scope_stack.clear_overlays();
    IntentResult::empty()
}

// --- Helpers ---

/// Checks whether the `#` at the cursor is in a valid position to trigger autocomplete.
fn is_valid_hash_trigger_position(input: &ChatInputBoxState) -> bool {
    let dollar_pos = input.cursor_pos() - 1;
    if dollar_pos == 0 {
        return true;
    }
    let prev = input.grapheme_at(dollar_pos - 1);
    prev == Some(" ") || prev == Some("\n")
}

/// Checks whether the `/` at the cursor is in a valid position to trigger slash autocomplete.
///
/// Valid only at position 0 (start of buffer).
fn is_valid_slash_trigger_position(input: &ChatInputBoxState) -> bool {
    input.cursor_pos() == 1 && input.text().starts_with('/')
}

/// Returns true if the cursor has moved outside the autocomplete token region,
/// requiring deactivation.
///
/// Deactivates when the cursor is before `token_start` or past `token_end`.
fn should_deactivate_on_cursor_move(state: &AppState) -> bool {
    let Some(ac) = state.active_chat_input().autocomplete() else {
        return false;
    };
    let cursor = state.active_chat_input().cursor_pos();
    let token_start = ac.token_start();
    if cursor <= token_start {
        return true;
    }
    let token_end = compute_token_end(state.active_chat_input(), token_start);
    cursor > token_end
}

/// Computes the grapheme index one past the last character of the token
/// that starts at `token_start` (the `#` position).
///
/// Scans forward from `token_start + 1` until whitespace, `#`, or end of buffer.
fn compute_token_end(input: &ChatInputBoxState, token_start: usize) -> usize {
    let graphemes: Vec<&str> = input.text().graphemes(true).collect();
    let len = graphemes.len();
    let mut end = token_start + 1;
    while end < len {
        let g = graphemes.get(end);
        if g.is_none() || g.is_some_and(|c| c.trim().is_empty() || *c == "#") {
            break;
        }
        end += 1;
    }
    end
}

/// Performs a fuzzy search against the prompt template store and returns matching entries.
fn compute_matches(store: &PromptTemplateStore, filter: &str) -> Vec<AutocompleteMatch> {
    store
        .fuzzy_search(filter)
        .into_iter()
        .map(|t| AutocompleteMatch {
            name: t.name.clone(),
            description: t.description.clone(),
        })
        .collect()
}

/// Computes matches for the active autocomplete based on its trigger kind.
fn compute_updated_matches(
    store: &PromptTemplateStore,
    trigger: Option<AutocompleteTrigger>,
    filter: &str,
) -> Vec<AutocompleteMatch> {
    match trigger {
        Some(AutocompleteTrigger::Slash) => compute_slash_matches(filter),
        _ => compute_matches(store, filter),
    }
}

/// Performs a fuzzy search against the slash command registry.
fn compute_slash_matches(filter: &str) -> Vec<AutocompleteMatch> {
    let filter_lower = filter.to_lowercase();
    let entries = SlashCommand::all_entries();
    entries
        .into_iter()
        .filter(|e| {
            if filter_lower.is_empty() {
                return true;
            }
            let name_lower = e.name.to_lowercase();
            // Simple fuzzy: check if all filter chars appear in order in the name.
            let mut filter_chars = filter_lower.chars().peekable();
            for c in name_lower.chars() {
                if Some(c) == filter_chars.peek().copied() {
                    filter_chars.next();
                }
            }
            filter_chars.peek().is_none()
        })
        .map(|e| AutocompleteMatch {
            name: e.name,
            description: e.description,
        })
        .collect()
}

/// Scans the buffer to detect if the cursor sits inside a `#token` region.
///
/// Returns `Some((token_start, filter_text))` if the cursor is within a valid
/// token, where `token_start` is the grapheme index of the `#` and `filter_text`
/// is the text between `#+1` and the cursor position.
fn find_hash_token_at_cursor(input: &ChatInputBoxState) -> Option<(usize, String)> {
    use unicode_segmentation::UnicodeSegmentation as _;

    let cursor = input.cursor_pos();
    let graphemes: Vec<&str> = input.text().graphemes(true).collect();
    let len = graphemes.len();

    // Scan leftward from the cursor to find a '#' at a valid trigger position.
    let mut i = cursor;
    loop {
        if graphemes.get(i) == Some(&"#") {
            // Check that the '#' is at a valid trigger position.
            let preceded_by_boundary = i == 0
                || graphemes.get(i.wrapping_sub(1)) == Some(&" ")
                || graphemes.get(i.wrapping_sub(1)) == Some(&"\n");
            if !preceded_by_boundary {
                return None;
            }
            // The token extends from i+1 to the next whitespace, '#', or end.
            let mut token_end = i + 1;
            while token_end < len {
                let g = graphemes.get(token_end);
                if g.is_none() || g.is_some_and(|c| c.trim().is_empty() || *c == "#") {
                    break;
                }
                token_end += 1;
            }
            // The cursor must be >= i (on the '#' or within the token) and <= token_end.
            if cursor >= i && cursor <= token_end {
                let filter: String = graphemes
                    .get((i + 1)..cursor)
                    .map(|s| s.join(""))
                    .unwrap_or_default();
                return Some((i, filter));
            }
            return None;
        }
        // If we hit whitespace going left, stop — no valid token.
        let g = graphemes.get(i);
        if g.is_some_and(|c| c.trim().is_empty()) {
            return None;
        }
        if i == 0 {
            return None;
        }
        i -= 1;
    }
}

/// Scans the buffer to detect if the cursor sits inside a `/command` region at position 0.
///
/// Returns `Some((token_start, filter_text))` if the buffer starts with `/` and the
/// cursor is within the token, where `token_start` is 0 and `filter_text` is the text
/// between position 1 and the cursor.
fn find_slash_token_at_cursor(input: &ChatInputBoxState) -> Option<(usize, String)> {
    use unicode_segmentation::UnicodeSegmentation as _;

    if !input.text().starts_with('/') {
        return None;
    }

    let cursor = input.cursor_pos();
    let graphemes: Vec<&str> = input.text().graphemes(true).collect();
    let len = graphemes.len();

    // The token extends from 1 to the next whitespace or end.
    let mut token_end = 1;
    while token_end < len {
        let g = graphemes.get(token_end);
        if g.is_none() || g.is_some_and(|c| c.trim().is_empty()) {
            break;
        }
        token_end += 1;
    }

    // The cursor must be >= 0 and <= token_end.
    if cursor <= token_end {
        let filter: String = graphemes
            .get(1..cursor)
            .map(|s| s.join(""))
            .unwrap_or_default();
        return Some((0, filter));
    }
    None
}

/// Attempts to re-activate autocomplete if the cursor sits inside a token region.
///
/// Checks for both `#token` and `/command` regions.
fn try_reactivate_autocomplete(state: &mut AppState) {
    if state.active_chat_input().autocomplete().is_some() {
        return;
    }

    // Try slash command token first (position 0).
    if let Some((token_start, filter)) = find_slash_token_at_cursor(state.active_chat_input()) {
        let matches = compute_slash_matches(&filter);
        state.active_chat_input_mut().activate_autocomplete(
            token_start,
            AutocompleteTrigger::Slash,
            matches,
        );
        return;
    }

    // Try hash token.
    let Some((token_start, filter)) = find_hash_token_at_cursor(state.active_chat_input()) else {
        return;
    };
    let matches = compute_matches(&state.context.prompt_templates, &filter);
    state.active_chat_input_mut().activate_autocomplete(
        token_start,
        AutocompleteTrigger::Hash,
        matches,
    );
}
