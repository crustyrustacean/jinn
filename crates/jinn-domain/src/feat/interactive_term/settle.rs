//! Settle detection and key encoding for interactive sessions.
//!
//! Two concerns:
//!
//! - **Settle** — after input (or spawn), the tool call must wait for the
//!   program to react before returning the screen. "Settled" means either
//!   no output for a quiet window (the program is idle) or the hard cap was
//!   hit (programs like `htop` animate forever and would never go quiet).
//!   The loop itself lives in the actor's `select!` (Phase 2); this module
//!   owns the decision logic (`should_settle`, deadlines) and the defaults.
//!
//! - **Encoding** — the model sends *named* keys (`"enter"`, `"ctrl+c"`,
//!   `"up"`); this module encodes them to the byte sequences a legacy-mode
//!   xterm terminal emits. The query responder deliberately leaves the Kitty
//!   keyboard protocol disabled, so legacy encoding is the right target.

use std::time::Duration;

/// Default quiet window: no output for this long ⇒ treat as settled.
pub const DEFAULT_QUIET_MS: u64 = 400;

/// Default hard cap on the settle wait, even with continuous output.
pub const DEFAULT_MAX_WAIT_MS: u64 = 3000;

/// A `Duration` for the default quiet window.
#[must_use]
pub fn default_quiet() -> Duration {
    Duration::from_millis(DEFAULT_QUIET_MS)
}

/// A `Duration` for the default settle cap.
#[must_use]
pub fn default_max_wait() -> Duration {
    Duration::from_millis(DEFAULT_MAX_WAIT_MS)
}

/// Whether the settle condition is met.
///
/// `quiet_for` is how long no output arrived; `waited` is the total time in
/// this settle wait. Settled when either bound is reached.
#[must_use]
pub fn should_settle(
    quiet_for: Duration,
    waited: Duration,
    quiet: Duration,
    cap: Duration,
) -> bool {
    quiet_for >= quiet || waited >= cap
}

/// The deadline for the quiet window, given the last output instant.
///
/// Encapsulated so the actor's select loop and tests share one definition of
/// "quiet deadline" instead of each recomputing it.
#[must_use]
pub fn quiet_deadline(last_output_at: std::time::Instant, quiet: Duration) -> std::time::Instant {
    last_output_at + quiet
}

/// How input arguments encode to pty bytes.
///
/// Emitted bytes are ordered: `text` verbatim, then each named key, then the
/// trailing `enter`. This mirrors the tool's argument semantics
/// (`interactive_term_send {text?, keys?, enter?}`).
#[must_use]
pub fn encode_input(text: Option<&str>, keys: &[String], enter: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if let Some(text) = text {
        out.extend_from_slice(text.as_bytes());
    }
    for key in keys {
        out.extend_from_slice(&encode_key(key));
    }
    if enter {
        out.push(b'\r');
    }
    out
}

/// Encodes one named key to its legacy-xterm byte sequence.
///
/// Recognized names (case-insensitive; `"c-"` prefixes for control):
/// `enter`/`return`, `esc`/`escape`, `tab`, `backspace`, `delete`/`del`,
/// `space`, `up`, `down`, `left`, `right`, `home`, `end`, `pageup`,
/// `pagedown`, `ctrl+<letter>`/`c-<letter>`, `alt+<key>`/`m-<key>` (ESC
/// prefix), and any single printable character verbatim. Unknown names
/// encode to nothing (empty bytes) so a typo'd key never sends garbage.
#[must_use]
pub fn encode_key(name: &str) -> Vec<u8> {
    let lower = name.trim().to_ascii_lowercase();
    let bytes: &[u8] = match lower.as_str() {
        "enter" | "return" | "\\n" | "\\r" => b"\r",
        "esc" | "escape" => b"\x1b",
        "tab" | "\\t" => b"\t",
        "backspace" => b"\x7f",
        "delete" | "del" => b"\x1b[3~",
        "space" => b" ",
        "up" | "uparrow" => b"\x1b[A",
        "down" | "downarrow" => b"\x1b[B",
        "right" | "rightarrow" => b"\x1b[C",
        "left" | "leftarrow" => b"\x1b[D",
        "home" => b"\x1b[H",
        "end" => b"\x1b[F",
        "pageup" => b"\x1b[5~",
        "pagedown" => b"\x1b[6~",
        _ if lower.starts_with("ctrl+") || lower.starts_with("c-") => return encode_ctrl(&lower),
        _ if lower.starts_with("alt+") || lower.starts_with("m-") => return encode_alt(&lower),
        // Single printable character, sent verbatim (case preserved —
        // `B` must reach a case-sensitive program as capital B).
        _ => {
            let trimmed = name.trim();
            let mut chars = trimmed.chars();
            match (chars.next(), chars.next()) {
                (Some(_), None) if !trimmed.starts_with('\\') => {
                    return trimmed.as_bytes().to_vec();
                }
                _ => {
                    // Literal newline/escape spellings already matched above;
                    // anything else multi-char is unknown → no bytes.
                    return Vec::new();
                }
            }
        }
    };
    bytes.to_vec()
}

/// Encodes `ctrl+<key>` / `c-<key>`: legacy C0 `key & 0x1F`.
fn encode_ctrl(spec: &str) -> Vec<u8> {
    let base = spec
        .strip_prefix("ctrl+")
        .or_else(|| spec.strip_prefix("c-"))
        .unwrap_or(spec);
    let mut chars = base.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None)
            if ch.is_ascii_alphabetic()
                || ch == '@'
                || ch == '['
                || ch == ']'
                || ch == '\\'
                || ch == '^'
                || ch == '_' =>
        {
            vec![(ch.to_ascii_uppercase() as u8) & 0x1f]
        }
        _ => Vec::new(),
    }
}

/// Encodes `alt+<key>` / `m-<key>`: ESC prefix followed by the key bytes.
fn encode_alt(spec: &str) -> Vec<u8> {
    let base = spec
        .strip_prefix("alt+")
        .or_else(|| spec.strip_prefix("m-"))
        .unwrap_or(spec);
    let mut out = vec![0x1b];
    out.extend_from_slice(&encode_key(base));
    out
}

/// Encodes a platform [`KeyEvent`] into the bytes a pty program expects.
///
/// This is the user-takeover counterpart of [`encode_key`]: the terminal
/// forwards raw key events in control mode, so encoding must match what a
/// real terminal sends — C0 controls for Ctrl, `ESC` prefix for Alt, uppercase
/// for Shift+letters, and standard `CSI ~` sequences for navigation keys.
#[must_use]
pub fn encode_key_event(event: &crate::protocol::key::KeyEvent) -> Vec<u8> {
    use crate::protocol::key::Key;

    let m = event.modifiers;
    let byte_for_char = |c: char| -> Vec<u8> {
        let mut bytes = c.to_string().into_bytes();
        match (m.ctrl, m.shift) {
            // Ctrl produces C0 controls; letters map A..=Z & 0x1F.
            (true, _) => {
                if let Some(b) = bytes.first_mut() {
                    *b = b.to_ascii_uppercase() & 0x1f;
                }
            }
            (false, true) => {
                for b in &mut bytes {
                    *b = b.to_ascii_uppercase();
                }
            }
            (false, false) => {}
        }
        if m.alt {
            let mut out = vec![0x1b];
            out.extend_from_slice(&bytes);
            return out;
        }
        bytes
    };

    let plain: &[u8] = match event.key {
        Key::Char(c) => return byte_for_char(c),
        Key::Enter => b"\r",
        Key::Esc => b"\x1b",
        Key::Tab => b"\t",
        Key::Backspace => b"\x7f",
        Key::Delete => b"\x1b[3~",
        Key::Up => b"\x1b[A",
        Key::Down => b"\x1b[B",
        Key::Right => b"\x1b[C",
        Key::Left => b"\x1b[D",
        Key::Home => b"\x1b[H",
        Key::End => b"\x1b[F",
        Key::PageUp => b"\x1b[5~",
        Key::PageDown => b"\x1b[6~",
        Key::F(n) => {
            // F1–F4 use the short SS3 form; F5+ use `CSI n ~`.
            return match n {
                1 => vec![0x1b, b'O', b'P'],
                2 => vec![0x1b, b'O', b'Q'],
                3 => vec![0x1b, b'O', b'R'],
                4 => vec![0x1b, b'O', b'S'],
                _ => {
                    // xterm codes: F5=15, F6=17, F7=18, F8=19, F9=20,
                    // F10=21, F11=23, F12=24.
                    let code = match n {
                        5 => 15,
                        6 => 17,
                        7 => 18,
                        8 => 19,
                        9 => 20,
                        10 => 21,
                        11 => 23,
                        12 => 24,
                        other => u32::from(other),
                    };
                    let mut out = b"\x1b[".to_vec();
                    out.extend_from_slice(code.to_string().as_bytes());
                    out.extend_from_slice(b"~");
                    out
                }
            };
        }
    };
    plain.to_vec()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;

    #[rstest::rstest]
    #[case("enter", b"\r")]
    #[case("return", b"\r")]
    #[case("esc", b"\x1b")]
    #[case("escape", b"\x1b")]
    #[case("tab", b"\t")]
    #[case("backspace", b"\x7f")]
    #[case("delete", b"\x1b[3~")]
    #[case("space", b" ")]
    #[case("up", b"\x1b[A")]
    #[case("down", b"\x1b[B")]
    #[case("right", b"\x1b[C")]
    #[case("left", b"\x1b[D")]
    #[case("home", b"\x1b[H")]
    #[case("end", b"\x1b[F")]
    #[case("pageup", b"\x1b[5~")]
    #[case("pagedown", b"\x1b[6~")]
    #[case("ctrl+c", &[0x03])]
    #[case("ctrl+d", &[0x04])]
    #[case("c-z", &[0x1a])]
    #[case("ctrl+a", &[0x01])]
    #[case("alt+x", b"\x1bx")]
    #[case("m-x", b"\x1bx")]
    #[case("q", b"q")]
    #[case("A", b"A")]
    #[case("a", b"a")]
    fn encode_key_maps_named_keys_to_xterm_bytes(#[case] name: &str, #[case] expected: &[u8]) {
        // Given a named key.
        // When encoding.
        let bytes = encode_key(name);
        // Then the legacy xterm sequence is produced.
        assert_eq!(bytes, expected, "encoding of {name:?}");
    }

    #[rstest::rstest]
    fn encode_key_is_case_insensitive_for_names() {
        // Given an upper-case key name.
        // When encoding.
        let bytes = encode_key("ENTER");
        // Then it matches the lower-case encoding.
        assert_eq!(bytes, b"\r");
    }

    #[rstest::rstest]
    fn unknown_key_name_encodes_to_nothing() {
        // Given a nonsense key name.
        // When encoding.
        let bytes = encode_key("frobnicate");
        // Then no bytes are produced (a typo never sends garbage).
        assert!(bytes.is_empty());
    }

    #[rstest::rstest]
    fn encode_input_orders_text_then_keys_then_enter() {
        // Given text, keys, and the enter flag.
        // When encoding all three.
        let bytes = encode_input(Some("ls -la"), &["tab".to_owned()], true);

        // Then the order is text, key, newline.
        assert_eq!(bytes, b"ls -la\t\r");
    }

    #[rstest::rstest]
    fn encode_input_with_only_text_sends_verbatim() {
        // Given only text.
        let bytes = encode_input(Some("pwd"), &[], false);
        // Then it is sent byte-for-byte.
        assert_eq!(bytes, b"pwd");
    }

    #[rstest::rstest]
    fn encode_input_with_nothing_sends_nothing() {
        // Given no inputs.
        let bytes = encode_input(None, &[], false);
        // Then nothing is sent (pure screen sync).
        assert!(bytes.is_empty());
    }

    #[rstest::rstest]
    fn should_settle_after_quiet_window() {
        // Given output stopped longer than the quiet window.
        let quiet_for = Duration::from_millis(500);
        let quiet = Duration::from_millis(400);
        let cap = Duration::from_secs(10);
        let waited = Duration::from_millis(600);

        // When checking the settle condition.
        // Then it is settled via the quiet bound.
        assert!(should_settle(quiet_for, waited, quiet, cap));
    }

    #[rstest::rstest]
    fn should_not_settle_while_output_is_fresh() {
        // Given output arrived 100ms ago (within the quiet window) and the
        // cap is far away.
        let quiet_for = Duration::from_millis(100);
        let quiet = Duration::from_millis(400);
        let cap = Duration::from_secs(10);
        let waited = Duration::from_millis(200);

        // Then it is not settled.
        assert!(!should_settle(quiet_for, waited, quiet, cap));
    }

    #[rstest::rstest]
    fn cap_wins_against_continuously_animating_program() {
        // Given output arriving constantly (never quiet) but the total wait
        // exceeded the cap (htop-style animation).
        let quiet_for = Duration::ZERO;
        let quiet = Duration::from_millis(400);
        let cap = Duration::from_secs(3);
        let waited = Duration::from_millis(3100);

        // Then it is settled via the cap.
        assert!(should_settle(quiet_for, waited, quiet, cap));
    }

    #[rstest::rstest]
    fn quiet_deadline_is_last_output_plus_quiet() {
        // Given a last-output instant.
        let last = std::time::Instant::now();
        let quiet = Duration::from_millis(400);

        // When computing the quiet deadline.
        let deadline = quiet_deadline(last, quiet);

        // Then it is exactly one quiet window after the last output.
        assert_eq!(deadline.duration_since(last), quiet);
    }

    #[rstest::rstest]
    #[case(crate::protocol::key::Key::Enter, crate::protocol::key::Modifiers::none(), &b"\r"[..])]
    #[case(
        crate::protocol::key::Key::Esc,
        crate::protocol::key::Modifiers::none(),
        b"\x1b"
    )]
    #[case(
        crate::protocol::key::Key::Up,
        crate::protocol::key::Modifiers::none(),
        b"\x1b[A"
    )]
    #[case(
        crate::protocol::key::Key::F(5),
        crate::protocol::key::Modifiers::none(),
        b"\x1b[15~"
    )]
    #[case(
        crate::protocol::key::Key::F(1),
        crate::protocol::key::Modifiers::none(),
        b"\x1bOP"
    )]
    fn encodes_plain_keys_from_events(
        #[case] key: crate::protocol::key::Key,
        #[case] modifiers: crate::protocol::key::Modifiers,
        #[case] expected: &[u8],
    ) {
        // Given a key event.
        let event = crate::protocol::key::KeyEvent { key, modifiers };

        // When encoding it for the pty.
        let bytes = encode_key_event(&event);

        // Then it matches the byte sequence a real terminal sends.
        assert_eq!(bytes, expected);
    }

    #[rstest::rstest]
    fn encodes_ctrl_char_as_c0_control() {
        // Given Ctrl+C.
        let event = crate::protocol::key::KeyEvent {
            key: crate::protocol::key::Key::Char('c'),
            modifiers: crate::protocol::key::Modifiers::ctrl(),
        };

        // When encoding it.
        let bytes = encode_key_event(&event);

        // Then it is the C0 ETX byte.
        assert_eq!(bytes, vec![0x03]);
    }

    #[rstest::rstest]
    fn encodes_alt_char_with_esc_prefix() {
        // Given Alt+X.
        let event = crate::protocol::key::KeyEvent {
            key: crate::protocol::key::Key::Char('x'),
            modifiers: crate::protocol::key::Modifiers::alt(),
        };

        // When encoding it.
        let bytes = encode_key_event(&event);

        // Then it is ESC followed by the key byte.
        assert_eq!(bytes, vec![0x1b, b'x']);
    }

    #[rstest::rstest]
    fn encodes_shift_char_as_uppercase() {
        // Given Shift+G (already normalized to 'G' by the TUI in practice).
        let event = crate::protocol::key::KeyEvent {
            key: crate::protocol::key::Key::Char('g'),
            modifiers: crate::protocol::key::Modifiers::shift(),
        };

        // When encoding it.
        let bytes = encode_key_event(&event);

        // Then the byte is uppercase.
        assert_eq!(bytes, b"G");
    }
}
