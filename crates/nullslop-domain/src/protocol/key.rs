//! Key representation for keyboard events.
//!
//! Backend-agnostic key types that decouple key handling
//! from any specific terminal library.

use serde::{Deserialize, Serialize};

/// Keyboard key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Key {
    /// A character key.
    Char(char),
    /// Enter key.
    Enter,
    /// Escape key.
    Esc,
    /// Tab key.
    Tab,
    /// Backspace key.
    Backspace,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Home key.
    Home,
    /// End key.
    End,
    /// Page up key.
    PageUp,
    /// Page down key.
    PageDown,
    /// Delete key (forward delete).
    Delete,
    /// Function key (F1–F12).
    F(u8),
}

#[cfg(feature = "which-key")]
impl ratatui_which_key::Key for KeyEvent {
    fn display(&self) -> String {
        if self.modifiers.ctrl
            && let Key::Char(c) = self.key
        {
            return format!("<C-{}>", c.to_ascii_lowercase());
        }

        let base = match self.key {
            Key::Char(' ') => "Space".to_owned(),
            Key::Char(c) => c.to_string(),
            Key::Tab => "Tab".to_owned(),
            Key::Enter => "Enter".to_owned(),
            Key::Backspace => "Backspace".to_owned(),
            Key::Esc => "Esc".to_owned(),
            Key::Up => "↑".to_owned(),
            Key::Down => "↓".to_owned(),
            Key::Left => "←".to_owned(),
            Key::Right => "→".to_owned(),
            Key::Home => "Home".to_owned(),
            Key::End => "End".to_owned(),
            Key::PageUp => "PageUp".to_owned(),
            Key::PageDown => "PageDown".to_owned(),
            Key::Delete => "Delete".to_owned(),
            Key::F(n) => format!("F{n}"),
        };

        match (self.modifiers.shift, self.modifiers.ctrl) {
            (true, false) => format!("S-{base}"),
            (false, true) => format!("C-{base}"),
            (true, true) => format!("C-S-{base}"),
            _ => base,
        }
    }

    fn is_backspace(&self) -> bool {
        matches!(self.key, Key::Backspace)
    }

    fn space() -> Self {
        KeyEvent {
            key: Key::Char(' '),
            modifiers: Modifiers::none(),
        }
    }

    fn from_char(c: char) -> Option<Self> {
        Some(KeyEvent {
            key: Key::Char(c),
            modifiers: Modifiers::none(),
        })
    }

    fn from_special_name(name: &str) -> Option<Self> {
        Self::parse_notation(name)
    }
}

impl KeyEvent {
    /// Parse a key notation string into a `KeyEvent`.
    ///
    /// Supports modifier-prefixed forms: `c-` for Ctrl and `s-` for Shift.
    /// Modifiers apply to both named keys and single characters.
    ///
    /// Named keys: `"tab"`, `"enter"`, `"escape"`, arrow keys,
    /// function keys (`"f1"`–`"f12"`), and symbolic aliases (`"lt"` → `<`, `"gt"` → `>`).
    ///
    /// Matching is case-insensitive.
    ///
    /// # Examples
    ///
    /// - `"c-x"` → Ctrl+X
    /// - `"s-enter"` → Shift+Enter
    /// - `"c-enter"` → Ctrl+Enter
    /// - `"tab"` → Tab
    /// - `"f5"` → F5
    /// - `"lt"` → <
    #[cfg(feature = "which-key")]
    pub fn parse_notation(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();

        let (modifiers, rest) = if let Some(stripped) = lower.strip_prefix("s-") {
            (Modifiers::shift(), stripped)
        } else if let Some(stripped) = lower.strip_prefix("c-") {
            (Modifiers::ctrl(), stripped)
        } else {
            (Modifiers::none(), lower.as_str())
        };

        let key = parse_key_name(rest)?;

        Some(KeyEvent { key, modifiers })
    }
}

/// Parse a lower-case key name string into a [`Key`].
///
/// Handles named keys (`"tab"`, `"enter"`, …), function keys (`"f1"`–`"f12"`),
/// symbolic aliases (`"lt"`, `"gt"`, `"space"`), and bare single characters.
#[cfg(feature = "which-key")]
fn parse_key_name(name: &str) -> Option<Key> {
    match name {
        "tab" => Some(Key::Tab),
        "enter" => Some(Key::Enter),
        "bs" | "backspace" => Some(Key::Backspace),
        "esc" | "escape" => Some(Key::Esc),
        "up" => Some(Key::Up),
        "down" => Some(Key::Down),
        "left" => Some(Key::Left),
        "right" => Some(Key::Right),
        "home" => Some(Key::Home),
        "end" => Some(Key::End),
        "pgup" | "pageup" => Some(Key::PageUp),
        "pgdn" | "pagedown" => Some(Key::PageDown),
        "delete" | "del" => Some(Key::Delete),
        "space" => Some(Key::Char(' ')),
        "lt" => Some(Key::Char('<')),
        "gt" => Some(Key::Char('>')),
        s if s.starts_with('f') && s.len() > 1 => {
            let num: u8 = s.get(1..)?.parse().ok()?;
            (1..=12).contains(&num).then_some(Key::F(num))
        }
        s if s.len() == 1 => Some(Key::Char(s.chars().next()?)),
        _ => None,
    }
}

/// Keyboard modifier flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Modifiers {
    /// Control key held.
    pub ctrl: bool,
    /// Alt key held.
    pub alt: bool,
    /// Shift key held.
    pub shift: bool,
}

impl Modifiers {
    /// Create a modifiers with no flags set.
    #[must_use]
    pub fn none() -> Self {
        Self {
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    /// Create a modifiers with only ctrl set.
    #[must_use]
    pub fn ctrl() -> Self {
        Self {
            ctrl: true,
            alt: false,
            shift: false,
        }
    }

    /// Create a modifiers with only alt set.
    #[must_use]
    pub fn alt() -> Self {
        Self {
            ctrl: false,
            alt: true,
            shift: false,
        }
    }

    /// Create a modifiers with only shift set.
    #[must_use]
    pub fn shift() -> Self {
        Self {
            ctrl: false,
            alt: false,
            shift: true,
        }
    }

    /// Returns `true` if no modifier flags are set.
    #[must_use]
    pub fn is_none(&self) -> bool {
        !self.ctrl && !self.alt && !self.shift
    }
}

/// A keyboard event with key and modifiers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyEvent {
    /// The key that was pressed.
    pub key: Key,
    /// Modifier keys held at the time.
    pub modifiers: Modifiers,
}
