//! `interactive_term` preferences — `[interactive_term]` in `jinn.toml`.
//!
//! Configures the takeover UI's control-toggle key and the tool-side settle
//! wait. Every field has a default, so the whole block is optional; unknown or
//! unusable control-toggle bindings fall back to the default with a warning
//! (degrade gracefully, never brick the terminal overlay).

use serde::{Deserialize, Serialize};

/// The default control-toggle key binding (`<c-g>`, matching the pi agent's
/// convention and near-unused by TUI programs). Toggles control mode in both
/// directions: view → control, and control → view.
pub const DEFAULT_CONTROL_TOGGLE_KEY: &str = "<c-g>";

/// The default quiet window (ms of silence before a send call returns).
pub const DEFAULT_SETTLE_QUIET_MS: u64 = 400;

/// The default hard cap (ms bounding any single settle wait).
pub const DEFAULT_SETTLE_MAX_WAIT_MS: u64 = 3000;

/// `[interactive_term]` preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractiveTermPrefs {
    /// Key that toggles terminal control mode in both directions: enters
    /// control mode from view mode, and exits control mode back to view mode.
    /// Notation follows the keymap (`<c-g>`, `<c-'>`, ...).
    #[serde(default = "default_control_toggle_key")]
    pub control_toggle_key: String,
    /// Milliseconds of output silence before a blocking call returns.
    #[serde(default = "default_settle_quiet_ms")]
    pub settle_quiet_ms: u64,
    /// Hard cap (ms) on any single settle wait, for programs that never
    /// stop repainting (htop, btop).
    #[serde(default = "default_settle_max_wait_ms")]
    pub settle_max_wait_ms: u64,
}

impl Default for InteractiveTermPrefs {
    fn default() -> Self {
        Self {
            control_toggle_key: DEFAULT_CONTROL_TOGGLE_KEY.to_owned(),
            settle_quiet_ms: DEFAULT_SETTLE_QUIET_MS,
            settle_max_wait_ms: DEFAULT_SETTLE_MAX_WAIT_MS,
        }
    }
}

/// Normalizes the configured control-toggle binding (trimmed), or `None`
/// when it is unusable (caller should fall back to the default).
///
/// Any binding the keybind system accepts is allowed — single keys
/// (`<c-g>`, `<m-g>`, `<f4>`, `'x'`) and sequences (`gg`) alike: validation
/// delegates to [`ratatui_which_key::parse_key_sequence`] with the same
/// [`KeyEvent`](crate::protocol::KeyEvent) the keymap binds through, so a
/// config value accepted here is guaranteed to bind. Modifier-name case is
/// irrelevant to parsing, so the raw spelling is returned unchanged.
///
/// A plain-key toggle (e.g. `g`) shadows that key in both directions — the
/// binding beats the forwarding catch-all — which is the user's explicit
/// choice via config.
#[must_use]
pub fn normalize_control_toggle_key(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // The leader argument only matters for `<leader>` expansions; the
    // toggle binds through the keymap's own parsing at bind time.
    let keys = ratatui_which_key::parse_key_sequence::<crate::protocol::KeyEvent>(
        trimmed,
        &<crate::protocol::KeyEvent as ratatui_which_key::Key>::space(),
    );
    if keys.is_empty() {
        // Unparseable tokens are silently dropped by the parser; an empty
        // result means nothing usable was written.
        return None;
    }
    Some(trimmed.to_owned())
}

/// Validates the prefs: falls back to defaults for unusable values.
///
/// Returns the corrected config to persist/use and whether any correction
/// happened (so the caller can warn).
#[must_use]
pub fn validated(prefs: &InteractiveTermPrefs) -> (InteractiveTermPrefs, bool) {
    let mut corrected = prefs.clone();
    let mut changed = false;
    if normalize_control_toggle_key(&prefs.control_toggle_key)
        .is_none_or(|norm| norm != prefs.control_toggle_key)
    {
        corrected.control_toggle_key = normalize_control_toggle_key(&prefs.control_toggle_key)
            .unwrap_or_else(|| DEFAULT_CONTROL_TOGGLE_KEY.to_owned());
        changed = true;
    }
    if prefs.settle_quiet_ms == 0 {
        corrected.settle_quiet_ms = DEFAULT_SETTLE_QUIET_MS;
        changed = true;
    }
    if prefs.settle_max_wait_ms < prefs.settle_quiet_ms {
        corrected.settle_max_wait_ms = prefs.settle_quiet_ms;
        changed = true;
    }
    (corrected, changed)
}

fn default_control_toggle_key() -> String {
    DEFAULT_CONTROL_TOGGLE_KEY.to_owned()
}

fn default_settle_quiet_ms() -> u64 {
    DEFAULT_SETTLE_QUIET_MS
}

fn default_settle_max_wait_ms() -> u64 {
    DEFAULT_SETTLE_MAX_WAIT_MS
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::*;

    #[rstest::rstest]
    #[case("<c-g>")]
    #[case("<m-g>")]
    #[case("<C-G>")]
    #[case("<c-'>")]
    #[case("<c-tab>")]
    #[case("<f4>")]
    #[case("<space>")]
    #[case("gg")]
    #[case("x")]
    fn accepts_any_keymap_parseable_binding(#[case] raw: &str) {
        // When normalizing a binding the keybind system can parse.
        let got = normalize_control_toggle_key(raw);

        // Then it is accepted verbatim (validation matches `bind()`).
        assert_eq!(got.as_deref(), Some(raw));
    }

    #[rstest::rstest]
    #[case("")]
    #[case("   ")]
    #[case("<junk>")]
    fn rejects_unusable_notations(#[case] raw: &str) {
        // When normalizing a binding the keybind system cannot parse.
        let got = normalize_control_toggle_key(raw);

        // Then it is rejected.
        assert_eq!(got, None);
    }

    #[rstest::rstest]
    fn validated_falls_back_to_default_for_bad_key() {
        // Given prefs with an unparseable control-toggle key.
        let prefs = InteractiveTermPrefs {
            control_toggle_key: "<junk>".to_owned(),
            ..InteractiveTermPrefs::default()
        };

        // When validating.
        let (corrected, changed) = validated(&prefs);

        // Then the key falls back to the default and the change is flagged.
        assert_eq!(corrected.control_toggle_key, DEFAULT_CONTROL_TOGGLE_KEY);
        assert!(changed);
    }

    #[rstest::rstest]
    fn validated_preserves_parseable_spellings() {
        // Given prefs with an unusual but parseable key spelling (the
        // keymap's parser is case-insensitive on modifier names).
        let prefs = InteractiveTermPrefs {
            control_toggle_key: "<C-G>".to_owned(),
            ..InteractiveTermPrefs::default()
        };

        // When validating.
        let (corrected, changed) = validated(&prefs);

        // Then the user's spelling is kept untouched.
        assert_eq!(corrected.control_toggle_key, "<C-G>");
        assert!(!changed);
    }

    #[rstest::rstest]
    fn validated_clamps_cap_below_quiet() {
        // Given prefs whose cap is below the quiet window.
        let prefs = InteractiveTermPrefs {
            settle_quiet_ms: 500,
            settle_max_wait_ms: 100,
            control_toggle_key: DEFAULT_CONTROL_TOGGLE_KEY.to_owned(),
        };

        // When validating.
        let (corrected, changed) = validated(&prefs);

        // Then the cap is raised to the quiet window.
        assert_eq!(corrected.settle_max_wait_ms, 500);
        assert!(changed);
    }

    #[rstest::rstest]
    fn validated_passes_sane_config_through() {
        // Given default prefs.
        let prefs = InteractiveTermPrefs::default();

        // When validating.
        let (corrected, changed) = validated(&prefs);

        // Then nothing changes.
        assert_eq!(corrected, prefs);
        assert!(!changed);
    }

    #[rstest::rstest]
    fn deserializes_from_empty_table() {
        // Given an empty `[interactive_term]` table.
        let raw = "[interactive_term]\n";

        // When deserializing.
        let prefs: InteractiveTermPrefs = toml::from_str(raw).expect("parse");

        // Then all defaults apply.
        assert_eq!(prefs, InteractiveTermPrefs::default());
    }

    #[rstest::rstest]
    fn serializes_renamed_field_and_roundtrips() {
        // Given default prefs.
        let prefs = InteractiveTermPrefs::default();

        // When serializing to TOML and parsing back.
        let raw = toml::to_string(&prefs).expect("serialize");
        let reparsed: InteractiveTermPrefs = toml::from_str(&raw).expect("parse");

        // Then the renamed field is written and survives the roundtrip.
        assert!(raw.contains("control_toggle_key"));
        assert!(!raw.contains("handback_key"));
        assert_eq!(reparsed, prefs);
    }
}
