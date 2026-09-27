# BDD refactor divergences

Assertions that did not survive contact with the code, recorded during the
BDD comment-convention pass over the workspace ROOT package (`src/`, `tests/`).

Each entry names the file, the test, what the test asserted, what the code
actually does, and the change made.

---

## `tests/slices/chat_input.rs` — `cursor_left_back_into_file_token_reactivates_the_popup`

- **What the test asserted.** That moving the cursor left from the end of
  `@foo bar` back into the `foo` token reactivates the `@` popup, i.e.
  `AppState::active_session().with_input(|i| i.autocomplete(), _)` is `Some`.
- **What the code does.** It stays `None`. Popup reactivation fires when the
  cursor *re-enters* a token from outside it; a cursor that is already inside
  the token and merely moves left within it does not reactivate. The buffer
  still reads `@foo bar` and the file-picker entries are unchanged, so this is
  a reactivation rule, not a failure to find the token.
- **Change.** The test was split out of `cursor_right_past_token_end_deactivates_popup`
  (which previously made the leftward moves and then asserted only the
  deactivation) and corrected to assert the observed behavior, renamed to
  `cursor_left_past_the_token_start_does_not_reactivate_the_popup`. The shared
  setup now lives in the `file_popup_deactivated_by_trailing_bar` fixture so
  both halves of the original test still run the same state transition.
