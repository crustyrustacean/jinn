//! `interactive_term` preferences — `[interactive_term]` in `jinn.toml`.
//!
//! Configures the takeover UI's handback key and the tool-side settle wait.
//! Every field has a default, so the whole block is optional; unknown or
//! unusable handback bindings fall back to the default with a warning
//! (degrade gracefully, never brick the terminal tab).

use serde::{Deserialize, Serialize};

/// The default handback key binding (`<c-g>`, matching the pi agent's
/// convention and near-unused by TUI programs).
pub const DEFAULT_HANDBACK_KEY: &str = "<c-g>";

/// The default quiet window (ms of silence before a send call returns).
pub const DEFAULT_SETTLE_QUIET_MS: u64 = 400;

/// The default hard cap (ms bounding any single settle wait).
pub const DEFAULT_SETTLE_MAX_WAIT_MS: u64 = 3000;

/// `[interactive_term]` preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractiveTermPrefs {
    /// Key that exits control mode and hands the terminal back to the agent.
    /// Notation follows the keymap (`<c-g>`, `<c-'>`, ...).
    #[serde(default = "default_handback_key")]
    pub handback_key: String,
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
            handback_key: DEFAULT_HANDBACK_KEY.to_owned(),
            settle_quiet_ms: DEFAULT_SETTLE_QUIET_MS,
            settle_max_wait_ms: DEFAULT_SETTLE_MAX_WAIT_MS,
        }
    }
}

/// Normalizes the configured handback binding to a lowercase, bracketed
/// form, or `None` when it is unusable (caller should fall back to the
/// default).
///
/// Accepts `<c-g>`, `c-g`, `<C-G>`; rejects empty strings, multi-key
/// sequences, and non-ctrl bindings (handback must not swallow plain keys).
#[must_use]
pub fn normalize_handback_key(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let inner = trimmed
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(trimmed);
    let lowered = inner.to_ascii_lowercase();
    let key = lowered.strip_prefix("c-")?;
    if key.is_empty() || key.len() > 1 {
        return None;
    }
    Some(format!("<c-{key}>"))
}

/// Validates the prefs: falls back to defaults for unusable values.
///
/// Returns the corrected config to persist/use and whether any correction
/// happened (so the caller can warn).
#[must_use]
pub fn validated(prefs: &InteractiveTermPrefs) -> (InteractiveTermPrefs, bool) {
    let mut corrected = prefs.clone();
    let mut changed = false;
    if normalize_handback_key(&prefs.handback_key)
        .is_none_or(|norm| norm != prefs.handback_key)
    {
        if normalize_handback_key(&prefs.handback_key).is_some() {
            corrected.handback_key =
                normalize_handback_key(&prefs.handback_key).unwrap_or_default();
        } else {
            corrected.handback_key = DEFAULT_HANDBACK_KEY.to_owned();
        }
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

fn default_handback_key() -> String {
    DEFAULT_HANDBACK_KEY.to_owned()
}

fn default_settle_quiet_ms() -> u64 {
    DEFAULT_SETTLE_QUIET_MS
}

fn default_settle_max_wait_ms() -> u64 {
    DEFAULT_SETTLE_MAX_WAIT_MS
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        reason = "test code"
    )]

    use super::*;

    #[rstest::rstest]
    #[case("<c-g>", Some("<c-g>"))]
    #[case("c-g", Some("<c-g>"))]
    #[case("<C-G>", Some("<c-g>"))]
    #[case("<c-'>", Some("<c-'>"))]
    fn normalizes_supported_notations(#[case] raw: &str, #[case] expected: Option<&str>) {
        // When normalizing the raw binding.
        let got = normalize_handback_key(raw);

        // Then it matches the canonical bracketed lowercase form.
        assert_eq!(got.as_deref(), expected);
    }

    #[rstest::rstest]
    #[case("")]
    #[case("g")]
    #[case("<c-tab>")]
    #[case("<m-g>")]
    fn rejects_unusable_notations(#[case] raw: &str) {
        // When normalizing an unusable binding.
        let got = normalize_handback_key(raw);

        // Then it is rejected.
        assert_eq!(got, None);
    }

    #[rstest::rstest]
    fn validated_falls_back_to_default_for_bad_key() {
        // Given prefs with a non-ctrl handback key.
        let prefs = InteractiveTermPrefs {
            handback_key: "g".to_owned(),
            ..InteractiveTermPrefs::default()
        };

        // When validating.
        let (corrected, changed) = validated(&prefs);

        // Then the key falls back to the default and the change is flagged.
        assert_eq!(corrected.handback_key, DEFAULT_HANDBACK_KEY);
        assert!(changed);
    }

    #[rstest::rstest]
    fn validated_normalizes_case_without_flagging_default() {
        // Given prefs with an unnormalized but usable key.
        let prefs = InteractiveTermPrefs {
            handback_key: "<C-G>".to_owned(),
            ..InteractiveTermPrefs::default()
        };

        // When validating.
        let (corrected, changed) = validated(&prefs);

        // Then the key is canonicalized and flagged for rewrite.
        assert_eq!(corrected.handback_key, "<c-g>");
        assert!(changed);
    }

    #[rstest::rstest]
    fn validated_clamps_cap_below_quiet() {
        // Given prefs whose cap is below the quiet window.
        let prefs = InteractiveTermPrefs {
            settle_quiet_ms: 500,
            settle_max_wait_ms: 100,
            handback_key: DEFAULT_HANDBACK_KEY.to_owned(),
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
}
