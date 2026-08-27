//! Verification helpers for default TOML config templates.
//!
//! Each default config file (e.g. `default_jinn.toml`) is embedded into the
//! binary via `include_str!` and shipped to users. To prevent silent drift
//! between the template and the corresponding struct's `Default`
//! implementation, every default template must be covered by a round-trip
//! check that deserializes the template and compares it to `T::default()`.
//!
//! This helper is deliberately tiny and generic so any future default config
//! (jinn.toml today; providers/themes later) can opt in with a one-line test.

use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fmt::Debug;
use wherror::Error;

/// Failures raised by [`check_default_round_trips_to_default`].
#[derive(Debug, Error)]
#[error(debug)]
pub enum DefaultConfigCheckError {
    /// The template is not valid TOML for the target type.
    Parse,
    /// The template parsed successfully but does not equal `T::default()`.
    Drift,
}

/// Verify a default TOML template round-trips to `T::default()`.
///
/// `T` must implement `DeserializeOwned + Serialize + Default + PartialEq +
/// Debug`. Comments, whitespace, key ordering, and integer/float formatting
/// differences in the template do not affect equality — only the *values*
/// carried by the deserialized struct are compared.
///
/// Catches three classes of drift between the template and the codebase:
///
/// - **Renamed field** — old name in template → falls back to default for the
///   new field, which may differ from the template's value → `Drift`.
/// - **Drifted default constant** — e.g. `DEFAULT_SLIDING_WINDOW_SIZE` changed
///   from 5 → 10 in code but template still says 5 → `Drift`.
/// - **Missing enum variant** — template's literal value is no longer a valid
///   variant → `Parse`. Or new variant became the default and template still
///   uses the old one → `Drift`.
///
/// # Errors
///
/// Returns [`DefaultConfigCheckError::Parse`] if the template cannot be
/// deserialized as `T`, or [`DefaultConfigCheckError::Drift`] if the parsed
/// value differs from `T::default()`.
pub fn check_default_round_trips_to_default<T>(
    template: &str,
) -> Result<(), DefaultConfigCheckError>
where
    T: DeserializeOwned + Serialize + Default + PartialEq + Debug,
{
    let parsed: T = toml::from_str(template).map_err(|_e| DefaultConfigCheckError::Parse)?;
    let default = T::default();
    if parsed == default {
        Ok(())
    } else {
        Err(DefaultConfigCheckError::Drift)
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unreachable,
        clippy::string_slice,
        clippy::uninlined_format_args,
        reason = "test code"
    )]
    use serde::Deserialize;
    use serde::Serialize;

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct SampleConfig {
        #[serde(default)]
        name: String,
        #[serde(default = "default_count")]
        count: u32,
    }

    impl Default for SampleConfig {
        fn default() -> Self {
            Self {
                name: String::new(),
                count: default_count(),
            }
        }
    }

    fn default_count() -> u32 {
        42
    }

    #[rstest::rstest]
    #[test]
    fn check_returns_ok_for_template_equal_to_default() {
        // Given a template that deserializes to SampleConfig::default().
        let template = "name = \"\"\ncount = 42\n";

        // When checking the template.
        let result = check_default_round_trips_to_default::<SampleConfig>(template);

        // Then the check passes.
        assert!(result.is_ok(), "expected Ok, got {result:?}");
    }

    #[rstest::rstest]
    #[test]
    fn check_returns_drift_err_for_mismatched_template() {
        // Given a template whose count differs from the default of 42.
        let template = "name = \"\"\ncount = 999\n";

        // When checking the template.
        let result = check_default_round_trips_to_default::<SampleConfig>(template);

        // Then the check returns Drift.
        assert!(matches!(result, Err(DefaultConfigCheckError::Drift)));
    }

    #[rstest::rstest]
    #[test]
    fn check_returns_parse_err_for_invalid_toml() {
        // Given a template with invalid TOML syntax.
        let template = "this is not = valid = toml";

        // When checking the template.
        let result = check_default_round_trips_to_default::<SampleConfig>(template);

        // Then the check returns Parse.
        assert!(matches!(result, Err(DefaultConfigCheckError::Parse)));
    }
}
