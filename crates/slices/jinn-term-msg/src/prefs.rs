//! Interactive-term control-toggle key normalization.
//!
//! The `[term]` section and its defaults live in
//! `jinn-preferences-config`; this module holds only the binding
//! normalizer, which is behavior and depends on the keybind parser.

/// Normalizes the configured control-toggle binding (trimmed), or `None`
/// when it is unusable (caller should fall back to the default).
///
/// Any binding the keybind system accepts is allowed — single keys
/// (`<c-g>`, `<m-g>`, `<f4>`, `'x'`) and sequences (`gg`) alike: validation
/// delegates to [`ratatui_which_key::parse_key_sequence`] with the same
/// [`KeyEvent`](jinn_slices::KeyEvent) the keymap binds through, so a
/// config value accepted here is guaranteed to bind. Modifier-name case is
/// irrelevant to parsing, so the raw spelling is returned unchanged.
#[must_use]
pub fn normalize_control_toggle_key(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let keys = ratatui_which_key::parse_key_sequence::<jinn_slices::KeyEvent>(
        trimmed,
        &<jinn_slices::KeyEvent as ratatui_which_key::Key>::space(),
    );
    if keys.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::normalize_control_toggle_key;

    #[rstest::rstest]
    #[case("<c-g>")]
    #[case(" <m-g> ")]
    #[case("<f4>")]
    fn accepts_bindings_the_keymap_binds(#[case] raw: &str) {
        // Given a well-formed control-toggle binding.
        // When normalizing it.
        let normalized = normalize_control_toggle_key(raw);
        // Then it survives (trimmed), so the keymap will accept it too.
        assert_eq!(normalized.as_deref(), Some(raw.trim()));
    }

    #[rstest::rstest]
    #[case("")]
    #[case("   ")]
    fn rejects_empty_bindings(#[case] raw: &str) {
        // Given an empty binding.
        // When normalizing it.
        // Then it is rejected (caller falls back to the default).
        assert!(normalize_control_toggle_key(raw).is_none());
    }
}
