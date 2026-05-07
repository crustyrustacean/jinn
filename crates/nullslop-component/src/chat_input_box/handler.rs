//! Handles user interactions with the chat input box.
//!
//! Responds to typing, deleting, clearing, and submitting messages, as well as
//! switching between normal (browsing) and input (typing) modes.
//!
//! When a message is submitted, it is enqueued via `EnqueueUserMessage` for
//! the message queue handler to dispatch. The input buffer is cleared immediately.
//!
//! Autocomplete: typing `$` at a valid position (start of buffer or preceded by a
//! space) activates prompt-template autocomplete. The popup shows fuzzy-matched
//! templates. Tab/Enter completes the name. Typing a closing `$` after an exact
//! match triggers double-`$` expansion.

use crate::AppState;
use crate::chat_input_box::AutocompleteMatch;
use crate::prompt_template::PromptTemplateStore;
use npr::CommandAction;
use npr::chat_input::{
    AutocompleteConfirm, Clear, DeleteGrapheme, DeleteGraphemeForward, InsertChar, Interrupt,
    MoveCursorDown, MoveCursorLeft, MoveCursorRight, MoveCursorToEnd, MoveCursorToStart,
    MoveCursorUp, MoveCursorWordLeft, MoveCursorWordRight, SubmitMessage,
};
use npr::system::SetMode;
use npr::tab::{SwitchTab, TabDirection};
use nullslop_component_core::{HandlerContext, define_handler};
use nullslop_protocol as npr;
use nullslop_services::Services;
use unicode_segmentation::UnicodeSegmentation as _;

define_handler! {
    pub(crate) struct ChatInputBoxHandler;

    commands {
        InsertChar: on_insert_char,
        DeleteGrapheme: on_delete_grapheme,
        DeleteGraphemeForward: on_delete_grapheme_forward,
        SubmitMessage: on_submit_message,
        Clear: on_clear,
        Interrupt: on_interrupt,
        MoveCursorLeft: on_move_cursor_left,
        MoveCursorRight: on_move_cursor_right,
        MoveCursorToStart: on_move_cursor_to_start,
        MoveCursorToEnd: on_move_cursor_to_end,
        MoveCursorWordLeft: on_move_cursor_word_left,
        MoveCursorWordRight: on_move_cursor_word_right,
        MoveCursorUp: on_move_cursor_up,
        MoveCursorDown: on_move_cursor_down,
        AutocompleteConfirm: on_autocomplete_confirm,
        SetMode: on_set_mode,
    }

    events {}
}

impl ChatInputBoxHandler {
    /// Inserts a character at the cursor position.
    ///
    /// Also handles `$` detection for autocomplete activation, space dismissal,
    /// and double-`$` expansion.
    fn on_insert_char(
        cmd: &InsertChar,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        let is_autocomplete_active = ctx.state.active_chat_input().autocomplete().is_some();

        if is_autocomplete_active {
            // Autocomplete is active — insert the character first.
            ctx.state
                .active_chat_input_mut()
                .insert_grapheme_at_cursor(cmd.ch);

            match cmd.ch {
                ' ' => {
                    // Space dismisses autocomplete.
                    ctx.state.active_chat_input_mut().deactivate_autocomplete();
                }
                '$' => {
                    // Check for double-$ expansion.
                    // The filter is the text BEFORE the just-inserted $,
                    // so we use cursor_pos - 1 as the end.
                    let Some(token_start) = ctx
                        .state
                        .active_chat_input()
                        .autocomplete_token_start()
                    else {
                        // Autocomplete was deactivated between checks.
                        return CommandAction::Continue;
                    };
                    let cursor_before_insert = ctx.state.active_chat_input().cursor_pos() - 1;
                    let filter: String = ctx
                        .state
                        .active_chat_input()
                        .text()
                        .graphemes(true)
                        .enumerate()
                        .skip_while(|(i, _)| *i < token_start + 1)
                        .take_while(|(i, _)| *i < cursor_before_insert)
                        .map(|(_, g)| g)
                        .collect();
                    if let Some(template) = ctx.state.prompt_templates.find_by_name(&filter) {
                        let body = template.body.clone();
                        ctx.state.active_chat_input_mut().expand_autocomplete(&body);
                    } else {
                        // No exact match — treat as literal, deactivate.
                        ctx.state.active_chat_input_mut().deactivate_autocomplete();
                    }
                }
                _ => {
                    // Regular character — update filter and recompute matches.
                    let filter = ctx
                        .state
                        .active_chat_input()
                        .autocomplete_filter()
                        .unwrap_or_default();
                    let matches = compute_matches(&ctx.state.prompt_templates, &filter);
                    ctx.state
                        .active_chat_input_mut()
                        .update_autocomplete_matches(matches);
                }
            }
        } else {
            // No autocomplete — just insert, then check if `$` triggers activation.
            ctx.state
                .active_chat_input_mut()
                .insert_grapheme_at_cursor(cmd.ch);

            if cmd.ch == '$' {
                let input = ctx.state.active_chat_input();
                if is_valid_trigger_position(input) {
                    let token_start = input.cursor_pos() - 1;
                    let matches = compute_matches(&ctx.state.prompt_templates, "");
                    ctx.state
                        .active_chat_input_mut()
                        .activate_autocomplete(token_start, matches);
                }
            }
        }

        CommandAction::Continue
    }

    /// Deletes the grapheme before the cursor.
    ///
    /// If autocomplete is active and the delete removes the `$` trigger,
    /// deactivates autocomplete. Otherwise updates the filter.
    fn on_delete_grapheme(
        _cmd: &DeleteGrapheme,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        let should_deactivate = if let Some(token_start) =
            ctx.state.active_chat_input().autocomplete_token_start()
        {
            ctx.state.active_chat_input().cursor_pos() <= token_start + 1
        } else {
            false
        };

        if should_deactivate {
            ctx.state.active_chat_input_mut().deactivate_autocomplete();
            ctx.state
                .active_chat_input_mut()
                .delete_grapheme_before_cursor();
        } else if ctx.state.active_chat_input().autocomplete().is_some() {
            // Within the filter region — delete and recompute.
            ctx.state
                .active_chat_input_mut()
                .delete_grapheme_before_cursor();
            let filter = ctx
                .state
                .active_chat_input()
                .autocomplete_filter()
                .unwrap_or_default();
            let matches = compute_matches(&ctx.state.prompt_templates, &filter);
            ctx.state
                .active_chat_input_mut()
                .update_autocomplete_matches(matches);
        } else {
            ctx.state
                .active_chat_input_mut()
                .delete_grapheme_before_cursor();
        }

        CommandAction::Continue
    }

    /// Submits the current input as a user message.
    ///
    /// If autocomplete is active, performs completion instead of submitting.
    fn on_submit_message(
        _cmd: &SubmitMessage,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        // If autocomplete is active, complete instead of submit.
        if ctx.state.active_chat_input().autocomplete().is_some() {
            if let Some(selected) = ctx.state.active_chat_input().autocomplete_selected() {
                let name = selected.name.clone();
                ctx.state
                    .active_chat_input_mut()
                    .complete_autocomplete(&name);
                let filter = ctx
                    .state
                    .active_chat_input()
                    .autocomplete_filter()
                    .unwrap_or_default();
                let matches = compute_matches(&ctx.state.prompt_templates, &filter);
                ctx.state
                    .active_chat_input_mut()
                    .update_autocomplete_matches(matches);
            }
            return CommandAction::Continue;
        }

        let text = ctx.state.active_chat_input().text().to_owned();
        if !text.is_empty() {
            let session_id = ctx.state.active_session.clone();
            ctx.state.active_chat_input_mut().reset();

            ctx.out.submit_command(npr::Command::EnqueueUserMessage {
                payload: npr::chat_input::EnqueueUserMessage { session_id, text },
            });
        }
        CommandAction::Continue
    }

    /// Clears the input buffer and resets the cursor.
    ///
    /// Deactivates autocomplete if active.
    fn on_clear(_cmd: &Clear, ctx: &mut HandlerContext<'_, AppState, Services>) -> CommandAction {
        ctx.state.active_chat_input_mut().deactivate_autocomplete();
        ctx.state.active_chat_input_mut().reset();
        CommandAction::Continue
    }

    /// Context-sensitive interrupt: clears the input buffer if non-empty, otherwise quits.
    ///
    /// Deactivates autocomplete if active.
    fn on_interrupt(
        _cmd: &Interrupt,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.active_chat_input_mut().deactivate_autocomplete();
        if ctx.state.active_chat_input().is_empty() {
            ctx.out.submit_command(npr::Command::Quit);
        } else {
            ctx.state.active_chat_input_mut().reset();
        }
        CommandAction::Continue
    }

    /// Sets the application input mode, cancelling active streams when leaving Input mode.
    fn on_set_mode(
        cmd: &SetMode,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        // When leaving Input mode during active streaming, cancel the stream.
        if ctx.state.mode == npr::Mode::Input
            && cmd.mode == npr::Mode::Normal
            && !ctx.state.active_session().is_idle()
        {
            let session_id = ctx.state.active_session.clone();
            ctx.out.submit_command(npr::Command::CancelStream {
                payload: npr::provider::CancelStream { session_id },
            });
        }

        // When leaving picker mode, clear the kind.
        if ctx.state.mode == npr::Mode::Picker && cmd.mode != npr::Mode::Picker {
            ctx.state.active_picker_kind = None;
        }

        ctx.state.mode = cmd.mode;
        CommandAction::Continue
    }

    /// Moves the cursor left one grapheme.
    ///
    /// If autocomplete is active and the cursor leaves the token region,
    /// deactivates autocomplete.
    fn on_move_cursor_left(
        _cmd: &MoveCursorLeft,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.active_chat_input_mut().move_cursor_left();
        let should_deactivate = {
            let input = ctx.state.active_chat_input();
            should_deactivate_on_cursor_move(input)
        };
        if should_deactivate {
            ctx.state.active_chat_input_mut().deactivate_autocomplete();
        }
        CommandAction::Continue
    }

    /// Moves the cursor right one grapheme.
    ///
    /// If autocomplete is active and the cursor leaves the token region,
    /// deactivates autocomplete.
    fn on_move_cursor_right(
        _cmd: &MoveCursorRight,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.active_chat_input_mut().move_cursor_right();
        let should_deactivate = {
            let input = ctx.state.active_chat_input();
            should_deactivate_on_cursor_move(input)
        };
        if should_deactivate {
            ctx.state.active_chat_input_mut().deactivate_autocomplete();
        }
        CommandAction::Continue
    }

    /// Moves the cursor to the beginning of the input.
    ///
    /// Deactivates autocomplete if active (cursor always leaves token region).
    fn on_move_cursor_to_start(
        _cmd: &MoveCursorToStart,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.active_chat_input_mut().deactivate_autocomplete();
        ctx.state.active_chat_input_mut().move_cursor_to_start();
        CommandAction::Continue
    }

    /// Moves the cursor to the end of the input.
    ///
    /// Deactivates autocomplete if active (cursor always leaves token region
    /// unless the token happens to be at the very end, but we still deactivate
    /// since the user explicitly moved to end).
    fn on_move_cursor_to_end(
        _cmd: &MoveCursorToEnd,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.active_chat_input_mut().deactivate_autocomplete();
        ctx.state.active_chat_input_mut().move_cursor_to_end();
        CommandAction::Continue
    }

    /// Deletes the grapheme after the cursor (forward delete).
    ///
    /// If autocomplete is active and the delete removes the `$` or goes
    /// beyond the filter region, deactivates autocomplete.
    fn on_delete_grapheme_forward(
        _cmd: &DeleteGraphemeForward,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        let token_start = ctx.state.active_chat_input().autocomplete_token_start();
        let cursor = ctx.state.active_chat_input().cursor_pos();

        if let Some(token_start) = token_start {
            if cursor == token_start {
                // Would delete the $ itself — deactivate and delete.
                ctx.state.active_chat_input_mut().deactivate_autocomplete();
                ctx.state
                    .active_chat_input_mut()
                    .delete_grapheme_after_cursor();
            } else {
                // Within or past filter — perform delete.
                ctx.state
                    .active_chat_input_mut()
                    .delete_grapheme_after_cursor();
                // Check if cursor is still in the token region.
                let should_deactivate = {
                    let input = ctx.state.active_chat_input();
                    should_deactivate_on_cursor_move(input)
                };
                if should_deactivate {
                    ctx.state.active_chat_input_mut().deactivate_autocomplete();
                } else {
                    let filter = ctx
                        .state
                        .active_chat_input()
                        .autocomplete_filter()
                        .unwrap_or_default();
                    let matches = compute_matches(&ctx.state.prompt_templates, &filter);
                    ctx.state
                        .active_chat_input_mut()
                        .update_autocomplete_matches(matches);
                }
            }
        } else {
            ctx.state
                .active_chat_input_mut()
                .delete_grapheme_after_cursor();
        }

        CommandAction::Continue
    }

    /// Moves the cursor left one word.
    ///
    /// Deactivates autocomplete (word-left always leaves the token region).
    fn on_move_cursor_word_left(
        _cmd: &MoveCursorWordLeft,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.active_chat_input_mut().deactivate_autocomplete();
        ctx.state.active_chat_input_mut().move_cursor_word_left();
        CommandAction::Continue
    }

    /// Moves the cursor right one word.
    ///
    /// Deactivates autocomplete (word-right always leaves the token region).
    fn on_move_cursor_word_right(
        _cmd: &MoveCursorWordRight,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state.active_chat_input_mut().deactivate_autocomplete();
        ctx.state.active_chat_input_mut().move_cursor_word_right();
        CommandAction::Continue
    }

    /// Moves the cursor up one visual line.
    ///
    /// If autocomplete is active, navigates the match list instead.
    fn on_move_cursor_up(
        _cmd: &MoveCursorUp,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        if ctx.state.active_chat_input().autocomplete().is_some() {
            ctx.state.active_chat_input_mut().autocomplete_move_up();
        } else {
            ctx.state.active_chat_input_mut().move_cursor_up();
        }
        CommandAction::Continue
    }

    /// Moves the cursor down one visual line.
    ///
    /// If autocomplete is active, navigates the match list instead.
    fn on_move_cursor_down(
        _cmd: &MoveCursorDown,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        if ctx.state.active_chat_input().autocomplete().is_some() {
            ctx.state.active_chat_input_mut().autocomplete_move_down();
        } else {
            ctx.state.active_chat_input_mut().move_cursor_down();
        }
        CommandAction::Continue
    }

    /// Confirms the autocomplete selection (Tab key in Input scope).
    ///
    /// When autocomplete is active, completes the selected template name.
    /// When inactive, falls back to `SwitchTab` so Tab still switches tabs.
    fn on_autocomplete_confirm(
        _cmd: &AutocompleteConfirm,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        if ctx.state.active_chat_input().autocomplete().is_some() {
            if let Some(selected) = ctx.state.active_chat_input().autocomplete_selected() {
                let name = selected.name.clone();
                ctx.state
                    .active_chat_input_mut()
                    .complete_autocomplete(&name);
                let filter = ctx
                    .state
                    .active_chat_input()
                    .autocomplete_filter()
                    .unwrap_or_default();
                let matches = compute_matches(&ctx.state.prompt_templates, &filter);
                ctx.state
                    .active_chat_input_mut()
                    .update_autocomplete_matches(matches);
            }
        } else {
            // Fallback: switch tab when autocomplete is inactive.
            ctx.out.submit_command(npr::Command::SwitchTab {
                payload: SwitchTab {
                    direction: TabDirection::Next,
                },
            });
        }
        CommandAction::Continue
    }
}

// --- Helper functions ---

/// Checks whether a `$` just typed at the current cursor position is at a valid
/// trigger position: start of buffer or preceded by a space.
///
/// The `$` is at `cursor_pos - 1` (just inserted, cursor advanced by 1).
fn is_valid_trigger_position(input: &crate::chat_input_box::ChatInputBoxState) -> bool {
    let dollar_pos = input.cursor_pos() - 1;
    if dollar_pos == 0 {
        return true;
    }
    input.grapheme_at(dollar_pos - 1) == Some(" ")
}

/// Computes autocomplete matches from the store for the given filter.
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

/// Returns `true` if the cursor has moved outside the `$`-token region
/// and autocomplete should be deactivated.
///
/// Deactivates when `cursor <= token_start`.
fn should_deactivate_on_cursor_move(input: &crate::chat_input_box::ChatInputBoxState) -> bool {
    let Some(ac) = input.autocomplete() else {
        return false;
    };
    input.cursor_pos() <= ac.token_start()
}

#[cfg(test)]
#[path = "handler_tests.rs"]
mod handler_tests;
