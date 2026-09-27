//! Translates raw terminal key events into application-level key events.
//!
//! Bridges platform-specific terminal input and the protocol key types
//! used by the keymap and command routing.

use jinn_kernel::{Key, KeyEvent, Modifiers};

/// Converts a platform key event to an application key event.
///
/// Returns `None` for key codes that have no application equivalent
/// (e.g., `KeyCode::Null`, `KeyCode::Modifier`).
#[must_use]
pub fn from_crossterm(event: crossterm::event::KeyEvent) -> Option<KeyEvent> {
    let (key, normalize_shift) = match event.code {
        crossterm::event::KeyCode::Char(c) => (Key::Char(c), true),
        crossterm::event::KeyCode::Enter => (Key::Enter, false),
        crossterm::event::KeyCode::Esc => (Key::Esc, false),
        // crossterm reports Shift+Tab as its own code rather than as
        // `Tab + SHIFT`. There is no separate `Key::BackTab`: the shift
        // lives in the modifiers, so `<s-tab>` bindings, the which-key
        // display and the pty encoder all reuse the existing modifier path.
        crossterm::event::KeyCode::Tab | crossterm::event::KeyCode::BackTab => (Key::Tab, false),
        crossterm::event::KeyCode::Backspace => (Key::Backspace, false),
        crossterm::event::KeyCode::Up => (Key::Up, false),
        crossterm::event::KeyCode::Down => (Key::Down, false),
        crossterm::event::KeyCode::Left => (Key::Left, false),
        crossterm::event::KeyCode::Right => (Key::Right, false),
        crossterm::event::KeyCode::Home => (Key::Home, false),
        crossterm::event::KeyCode::End => (Key::End, false),
        crossterm::event::KeyCode::PageUp => (Key::PageUp, false),
        crossterm::event::KeyCode::PageDown => (Key::PageDown, false),
        crossterm::event::KeyCode::Delete => (Key::Delete, false),
        crossterm::event::KeyCode::F(n) => (Key::F(n), false),
        _ => return None,
    };

    let has_shift = event
        .modifiers
        .contains(crossterm::event::KeyModifiers::SHIFT)
        // BackTab *is* Shift+Tab, so the modifier is implied by the code
        // itself. Deriving it from the code rather than trusting the
        // modifier keeps a terminal that reports BackTab without SHIFT from
        // collapsing the keystroke into a plain Tab.
        || matches!(event.code, crossterm::event::KeyCode::BackTab);

    // Normalize: terminals differ in how they represent Shift + letter keys.
    // Some send Char('g') + SHIFT, others send Char('G') + SHIFT.
    // The keymap binds "G" as Key::Char('G') with no modifiers.
    // Normalize both cases to Key::Char('G') with shift cleared.
    let (key, has_shift) = if normalize_shift && has_shift {
        if let Key::Char(c) = key {
            if c.is_ascii_lowercase() {
                // shift+'g' → 'G', clear shift
                (Key::Char(c.to_ascii_uppercase()), false)
            } else if c.is_ascii_uppercase() {
                // shift+'G' → 'G', clear shift (already uppercase)
                (Key::Char(c), false)
            } else {
                (Key::Char(c), true)
            }
        } else {
            (key, true)
        }
    } else {
        (key, has_shift)
    };

    let modifiers = Modifiers {
        ctrl: event
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL),
        alt: event
            .modifiers
            .contains(crossterm::event::KeyModifiers::ALT),
        shift: has_shift,
    };

    Some(KeyEvent { key, modifiers })
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "test code, panics are acceptable"
    )]
    use super::*;

    fn crossterm_key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    fn crossterm_key_with_mod(
        code: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
    ) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, modifiers)
    }

    #[rstest::rstest]
    fn convert_char_key() {
        // Given crossterm Char('a').
        let event = crossterm_key(crossterm::event::KeyCode::Char('a'));

        // When converting.
        let result = from_crossterm(event);

        // Then returns Key::Char('a') with no modifiers.
        let key_event = result.expect("should convert");
        assert_eq!(key_event.key, Key::Char('a'));
        assert!(key_event.modifiers.is_none());
    }

    #[rstest::rstest]
    fn convert_ctrl_enter() {
        // Given crossterm Enter with CONTROL.
        let event = crossterm_key_with_mod(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::CONTROL,
        );

        // When converting.
        let result = from_crossterm(event);

        // Then returns Key::Enter with ctrl=true.
        let key_event = result.expect("should convert");
        assert_eq!(key_event.key, Key::Enter);
        assert!(key_event.modifiers.ctrl);
    }

    #[rstest::rstest]
    fn convert_f_key() {
        // Given crossterm F(5).
        let event = crossterm_key(crossterm::event::KeyCode::F(5));

        // When converting.
        let result = from_crossterm(event);

        // Then returns Key::F(5).
        let key_event = result.expect("should convert");
        assert_eq!(key_event.key, Key::F(5));
    }

    #[rstest::rstest]
    fn convert_shift_lowercase_g_produces_uppercase_g() {
        // Given crossterm Shift+Char('g').
        let event = crossterm_key_with_mod(
            crossterm::event::KeyCode::Char('g'),
            crossterm::event::KeyModifiers::SHIFT,
        );

        // When converting.
        let result = from_crossterm(event);

        // Then returns Key::Char('G') with no shift modifier.
        let key_event = result.expect("should convert");
        assert_eq!(key_event.key, Key::Char('G'));
        assert!(!key_event.modifiers.shift);
        assert!(key_event.modifiers.is_none());
    }

    #[rstest::rstest]
    fn convert_shift_lowercase_a_produces_uppercase_a() {
        // Given crossterm Shift+Char('a').
        let event = crossterm_key_with_mod(
            crossterm::event::KeyCode::Char('a'),
            crossterm::event::KeyModifiers::SHIFT,
        );

        // When converting.
        let result = from_crossterm(event);

        // Then returns Key::Char('A') with no shift modifier.
        let key_event = result.expect("should convert");
        assert_eq!(key_event.key, Key::Char('A'));
        assert!(!key_event.modifiers.shift);
    }

    #[rstest::rstest]
    fn convert_direct_uppercase_g_no_shift() {
        // Given crossterm Char('G') with no modifiers.
        let event = crossterm_key(crossterm::event::KeyCode::Char('G'));

        // When converting.
        let result = from_crossterm(event);

        // Then returns Key::Char('G') with no modifiers.
        let key_event = result.expect("should convert");
        assert_eq!(key_event.key, Key::Char('G'));
        assert!(key_event.modifiers.is_none());
    }

    #[rstest::rstest]
    fn convert_shift_uppercase_g_clears_shift() {
        // Given crossterm Char('G') with SHIFT (some terminals send this).
        let event = crossterm_key_with_mod(
            crossterm::event::KeyCode::Char('G'),
            crossterm::event::KeyModifiers::SHIFT,
        );

        // When converting.
        let result = from_crossterm(event);

        // Then returns Key::Char('G') with shift cleared.
        let key_event = result.expect("should convert");
        assert_eq!(key_event.key, Key::Char('G'));
        assert!(!key_event.modifiers.shift);
        assert!(key_event.modifiers.is_none());
    }

    #[rstest::rstest]
    fn convert_shift_enter_preserves_shift_modifier() {
        // Given crossterm Enter with SHIFT.
        let event = crossterm_key_with_mod(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::SHIFT,
        );

        // When converting.
        let result = from_crossterm(event);

        // Then returns Key::Enter with shift=true.
        let key_event = result.expect("should convert");
        assert_eq!(key_event.key, Key::Enter);
        assert!(key_event.modifiers.shift);
        assert!(!key_event.modifiers.ctrl);
    }

    #[rstest::rstest]
    fn convert_ctrl_enter_preserves_ctrl_modifier() {
        // Given crossterm Enter with CONTROL.
        let event = crossterm_key_with_mod(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::CONTROL,
        );

        // When converting.
        let result = from_crossterm(event);

        // Then returns Key::Enter with ctrl=true.
        let key_event = result.expect("should convert");
        assert_eq!(key_event.key, Key::Enter);
        assert!(key_event.modifiers.ctrl);
        assert!(!key_event.modifiers.shift);
    }

    #[rstest::rstest]
    fn convert_plain_enter_has_no_modifiers() {
        // Given crossterm Enter with no modifiers.
        let event = crossterm_key(crossterm::event::KeyCode::Enter);

        // When converting.
        let result = from_crossterm(event);

        // Then returns Key::Enter with no modifiers.
        let key_event = result.expect("should convert");
        assert_eq!(key_event.key, Key::Enter);
        assert!(key_event.modifiers.is_none());
    }

    /// crossterm reports Shift+Tab as `KeyCode::BackTab`, not as
    /// `Tab + SHIFT`. Without an arm for it the event fell through to the
    /// `_ => return None` arm and the keystroke was discarded outright.
    #[rstest::rstest]
    fn convert_back_tab_becomes_tab_with_shift() {
        // Given crossterm BackTab with SHIFT.
        let event = crossterm_key_with_mod(
            crossterm::event::KeyCode::BackTab,
            crossterm::event::KeyModifiers::SHIFT,
        );

        // When converting.
        let result = from_crossterm(event);

        // Then returns Key::Tab with shift=true.
        let key_event = result.expect("should convert");
        assert_eq!(key_event.key, Key::Tab);
        assert!(key_event.modifiers.shift);
        // And no other modifier is set.
        assert!(!key_event.modifiers.ctrl);
        assert!(!key_event.modifiers.alt);
    }

    /// BackTab implies shift by definition, so the conversion derives it from
    /// the key code rather than trusting the modifier bits. A terminal that
    /// reports BackTab without SHIFT must still not collapse into a plain Tab,
    /// which would be sent to the child as a horizontal tab.
    #[rstest::rstest]
    fn convert_back_tab_without_shift_modifier_still_preserves_shift() {
        // Given crossterm BackTab with no modifiers set.
        let event = crossterm_key(crossterm::event::KeyCode::BackTab);

        // When converting.
        let result = from_crossterm(event);

        // Then returns Key::Tab with shift=true.
        let key_event = result.expect("should convert");
        assert_eq!(key_event.key, Key::Tab);
        assert!(key_event.modifiers.shift);
    }

    #[rstest::rstest]
    fn convert_plain_tab_has_no_shift() {
        // Given crossterm Tab with no modifiers.
        let event = crossterm_key(crossterm::event::KeyCode::Tab);

        // When converting.
        let result = from_crossterm(event);

        // Then returns Key::Tab with no shift — the BackTab implication must
        // not bleed into a genuine Tab.
        let key_event = result.expect("should convert");
        assert_eq!(key_event.key, Key::Tab);
        assert!(key_event.modifiers.is_none());
    }

    #[rstest::rstest]
    fn convert_unknown_returns_none() {
        // Given crossterm Null.
        let event = crossterm_key(crossterm::event::KeyCode::Null);

        // When converting.
        let result = from_crossterm(event);

        // Then returns None.
        assert_eq!(result, None);
    }
}
