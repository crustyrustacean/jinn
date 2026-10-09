//! Graph-name validation: the config table name is the server identity, the
//! tool argument is the on-disk file, and the two must stay filesystem-safe.

use error_stack::Report;

/// Validates a graph name to filesystem-safe `[a-z0-9_-]`.
///
/// The name becomes `<name>.graph` under the minigraf directory; anything that
/// could escape that directory (path separators, `..`, leading dots) or that
/// would render ambiguously on the wire (uppercase, unicode, whitespace) is
/// rejected with the offending name carried back.
///
/// # Errors
///
/// Returns [`MinigrafServerError::BadName`] describing why the name is rejected.
pub fn validate(name: &str) -> Result<(), Report<crate::errors::MinigrafServerError>> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(error_stack::Report::new(
            crate::errors::MinigrafServerError::BadName(format!(
                "`{name}` must be 1-64 chars of [a-z0-9_-]"
            )),
        ))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test assertions")]

    use super::*;

    #[rstest::rstest]
    #[case::plain("workflows")]
    #[case::digits("graph2")]
    #[case::hyphen("my-graph")]
    #[case::underscore("my_graph")]
    #[case::mixed("a1-b2_c3")]
    fn accepts_filesystem_safe_names(#[case] name: &str) {
        // Given a name built from [a-z0-9_-].
        // When validating.
        // Then it passes.
        assert!(validate(name).is_ok(), "{name} should validate");
    }

    #[rstest::rstest]
    #[case::empty("")]
    #[case::uppercase("Workflows")]
    #[case::space("my graph")]
    #[case::dot("my.graph")]
    #[case::traversal("../etc")]
    #[case::slash("a/b")]
    #[case::leading_dot(".hidden")]
    #[case::unicode("gräf")]
    #[case::too_long("a".repeat(65))]
    fn rejects_unsafe_names(#[case] name: String) {
        // Given a name outside [a-z0-9_-] (or over 64 chars).
        // When validating.
        let result = validate(&name);

        // Then it fails naming the name.
        let err = result.expect_err("must reject");
        let rendered = format!("{err:?}");
        assert!(
            rendered.contains("invalid graph name"),
            "expected the name rule in: {rendered}"
        );
    }
}
