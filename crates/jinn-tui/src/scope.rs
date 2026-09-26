//! Keymap scopes for context-sensitive key handling.
//!
//! The scope determines which set of keybindings is active.
//! Each sidebar section has its own scope so section-specific keys
//! (like `r` for rename vs pin-relative) are unambiguous.

/// The current keymap context.
///
/// Controls which keybindings are active. Set via
/// [`ratatui_which_key::WhichKeyState::set_scope`].
///
/// Static variants are the composition-owned scopes. A slice's dynamic
/// scope ([`Scope::Dynamic`]) carries its identity as data, so slices
/// never edit this enum; their keymap bindings are generated from
/// registered route rows.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    /// Normal mode - navigation and commands.
    Normal,
    /// Input mode - typing into the input buffer.
    Input,
    /// A dynamically-registered slice's scope.
    ///
    /// Derives `Ord` on the inner string-based id (which-key stores
    /// catch-all handlers in a `BTreeMap<S, _>`), so the derived
    /// ordering is required, not hand-rolled.
    Dynamic(jinn_slices::SliceScopeId),
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normal => write!(f, "Normal"),

            Self::Input => write!(f, "Input"),
            Self::Dynamic(id) => write!(f, "dynamic:{id}"),
        }
    }
}

impl std::str::FromStr for Scope {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Dynamic scopes parse first: `dynamic:<slice>:<name>` is the
        // Display inverse and must round-trip.
        if let Some(rest) = s.strip_prefix("dynamic:") {
            let id = rest.parse::<jinn_slices::SliceScopeId>()?;
            return Ok(Self::Dynamic(id));
        }
        match s {
            "Normal" => Ok(Self::Normal),

            "Input" => Ok(Self::Input),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::Scope;
    use std::str::FromStr;

    #[rstest::rstest]
    #[test]
    fn dynamic_scope_round_trips() {
        // Given a dynamic scope carrying a slice scope id.
        let scope = Scope::Dynamic(jinn_slices::SliceScopeId::new("quake-bar", "open"));

        // When formatting then parsing back.
        let s = scope.to_string();
        // Then the display form is the `dynamic:` prefixed key.
        assert_eq!(s, "dynamic:quake-bar:open");
        assert_eq!(
            Scope::from_str(&s),
            Ok(scope),
            "Display/FromStr should round-trip"
        );
    }
}
