#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use crate::protocol::key::Modifiers;
use crate::{Key, KeyEvent};

#[rstest::rstest]
#[case::ctrl(Modifiers::none(), "ctrl", false)]
#[case::alt(Modifiers::none(), "alt", false)]
#[case::shift(Modifiers::none(), "shift", false)]
fn modifiers_none_flag_is_false(
    #[case] mods: Modifiers,
    #[case] flag: &str,
    #[case] expected: bool,
) {
    // Given modifiers created with none().
    // When inspecting each flag.
    // Then each individual flag is false.
    let actual = match flag {
        "ctrl" => mods.ctrl,
        "alt" => mods.alt,
        "shift" => mods.shift,
        _ => panic!("unknown flag: {flag}"),
    };
    assert_eq!(actual, expected);
}

#[rstest::rstest]
fn parse_notation_s_enter_returns_shift_enter() {
    // Given the notation "s-enter".
    let result = KeyEvent::parse_notation("s-enter");

    // When parsing.
    let key_event = result.expect("should parse");

    // Then it is Shift+Enter.
    assert_eq!(key_event.key, Key::Enter);
    assert!(key_event.modifiers.shift);
    assert!(!key_event.modifiers.ctrl);
}

#[rstest::rstest]
fn parse_notation_c_enter_returns_ctrl_enter() {
    // Given the notation "c-enter".
    let result = KeyEvent::parse_notation("c-enter");

    // When parsing.
    let key_event = result.expect("should parse");

    // Then it is Ctrl+Enter.
    assert_eq!(key_event.key, Key::Enter);
    assert!(key_event.modifiers.ctrl);
    assert!(!key_event.modifiers.shift);
}

#[rstest::rstest]
fn parse_notation_enter_returns_unmodified() {
    // Given the notation "enter".
    let result = KeyEvent::parse_notation("enter");

    // When parsing.
    let key_event = result.expect("should parse");

    // Then it is plain Enter with no modifiers.
    assert_eq!(key_event.key, Key::Enter);
    assert!(key_event.modifiers.is_none());
}

// --- parse_key_name coverage (via parse_notation) ---

#[rstest::rstest]
#[case::tab("tab", Key::Tab)]
#[case::enter("enter", Key::Enter)]
#[case::bs("bs", Key::Backspace)]
#[case::backspace("backspace", Key::Backspace)]
#[case::esc("esc", Key::Esc)]
#[case::escape("escape", Key::Esc)]
#[case::up("up", Key::Up)]
#[case::down("down", Key::Down)]
#[case::left("left", Key::Left)]
#[case::right("right", Key::Right)]
#[case::home("home", Key::Home)]
#[case::end("end", Key::End)]
#[case::pgup("pgup", Key::PageUp)]
#[case::pageup("pageup", Key::PageUp)]
#[case::pgdn("pgdn", Key::PageDown)]
#[case::pagedown("pagedown", Key::PageDown)]
#[case::delete("delete", Key::Delete)]
#[case::del("del", Key::Delete)]
#[case::space("space", Key::Char(' '))]
#[case::lt("lt", Key::Char('<'))]
#[case::gt("gt", Key::Char('>'))]
fn parse_notation_named_keys(#[case] input: &str, #[case] expected: Key) {
    // Given a notation string for a named key.
    let result = KeyEvent::parse_notation(input);

    // When parsing.
    let key_event = result.expect("should parse");

    // Then the key matches with no modifiers.
    assert_eq!(key_event.key, expected);
    assert!(key_event.modifiers.is_none());
}

#[rstest::rstest]
#[case::f1("f1", 1)]
#[case::f6("f6", 6)]
#[case::f12("f12", 12)]
fn parse_notation_function_keys(#[case] input: &str, #[case] num: u8) {
    // Given a function key notation.
    let result = KeyEvent::parse_notation(input);

    // When parsing.
    let key_event = result.expect("should parse");

    // Then it is the correct function key.
    assert_eq!(key_event.key, Key::F(num));
    assert!(key_event.modifiers.is_none());
}

#[rstest::rstest]
#[case::f0("f0")]
#[case::f13("f13")]
fn parse_notation_rejects_out_of_range_function_keys(#[case] input: &str) {
    // Given an out-of-range function key notation.
    let result = KeyEvent::parse_notation(input);

    // When parsing.

    // Then it returns None.
    assert!(result.is_none());
}

#[rstest::rstest]
fn parse_notation_single_char_returns_key_event() {
    // Given a single-character notation.
    let result = KeyEvent::parse_notation("a");

    // When parsing.
    let key_event = result.expect("should parse");

    // Then it is Char('a') with no modifiers.
    assert_eq!(key_event.key, Key::Char('a'));
    assert!(key_event.modifiers.is_none());
}

#[rstest::rstest]
fn parse_notation_ctrl_single_char() {
    // Given a ctrl-modified single-char notation.
    let result = KeyEvent::parse_notation("c-x");

    // When parsing.
    let key_event = result.expect("should parse");

    // Then it is Ctrl+Char('x').
    assert_eq!(key_event.key, Key::Char('x'));
    assert!(key_event.modifiers.ctrl);
    assert!(!key_event.modifiers.shift);
}

#[rstest::rstest]
fn parse_notation_shift_single_char() {
    // Given a shift-modified single-char notation.
    let result = KeyEvent::parse_notation("s-a");

    // When parsing.
    let key_event = result.expect("should parse");

    // Then it is Shift+Char('a').
    assert_eq!(key_event.key, Key::Char('a'));
    assert!(!key_event.modifiers.ctrl);
    assert!(key_event.modifiers.shift);
}

#[rstest::rstest]
fn parse_notation_case_insensitive() {
    // Given a notation with mixed case.
    let result = KeyEvent::parse_notation("ENTER");

    // When parsing.
    let key_event = result.expect("should parse");

    // Then it still resolves correctly.
    assert_eq!(key_event.key, Key::Enter);
}

#[rstest::rstest]
#[case::empty("")]
#[case::unknown("foobar")]
#[case::bare_ctrl("c-")]
#[case::bare_shift("s-")]
#[case::bare_meta("m-")]
fn parse_notation_rejects_invalid_inputs(#[case] input: &str) {
    // Given an invalid notation.
    let result = KeyEvent::parse_notation(input);

    // When parsing.

    // Then it returns None.
    assert!(result.is_none());
}

#[rstest::rstest]
fn parse_notation_m_s_returns_alt_s() {
    // Given the notation "m-s".
    let result = KeyEvent::parse_notation("m-s");

    // When parsing.
    let key_event = result.expect("should parse");

    // Then it is Alt+Char('s').
    assert_eq!(key_event.key, Key::Char('s'));
    assert!(key_event.modifiers.alt);
    assert!(!key_event.modifiers.ctrl);
    assert!(!key_event.modifiers.shift);
}

#[rstest::rstest]
fn parse_notation_m_enter_returns_alt_enter() {
    // Given the notation "m-enter".
    let result = KeyEvent::parse_notation("m-enter");

    // When parsing.
    let key_event = result.expect("should parse");

    // Then it is Alt+Enter.
    assert_eq!(key_event.key, Key::Enter);
    assert!(key_event.modifiers.alt);
    assert!(!key_event.modifiers.ctrl);
    assert!(!key_event.modifiers.shift);
}

#[rstest::rstest]
fn display_alt_char_shows_m_notation() {
    // Given a KeyEvent with Alt+Char('s').
    use ratatui_which_key::Key as _;
    let key_event = KeyEvent {
        key: Key::Char('s'),
        modifiers: Modifiers::alt(),
    };

    // When displaying.
    let display = key_event.display();

    // Then it shows "<M-s>".
    assert_eq!(display, "<M-s>");
}

#[rstest::rstest]
fn display_alt_named_key_shows_m_prefix() {
    // Given a KeyEvent with Alt+Enter.
    use ratatui_which_key::Key as _;
    let key_event = KeyEvent {
        key: Key::Enter,
        modifiers: Modifiers::alt(),
    };

    // When displaying.
    let display = key_event.display();

    // Then it shows "M-Enter".
    assert_eq!(display, "M-Enter");
}
