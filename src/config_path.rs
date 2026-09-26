//! Which `jinn.toml` a run reads and writes.
//!
//! `jinn.toml` normally lives at the platform's per-user location
//! (`~/.config/jinn/jinn.toml` on Linux). A run may instead point
//! `--config` at a different file, which redirects *both* reads and
//! writes for that run so the layer stays a single coherent source of
//! truth.
//!
//! The two cases differ deliberately:
//!
//! - The default path is a first-install affordance — nobody typed it, so
//!   a missing file is seeded from the embedded template.
//! - An explicit `--config` path is a user reference. If it does not
//!   exist, that is almost certainly a typo, and silently starting from
//!   defaults is a failure worth surfacing instead. Nothing is created.
//!
//! [`resolve_config_path`] only *decides*; it never writes. Seeding is a
//! separate step, which keeps this function pure and its tests free of
//! filesystem mutation beyond setup.

use std::path::{Path, PathBuf};

use error_stack::Report;
use wherror::Error;

/// The `--config` flag names a file that does not exist.
///
/// The path is carried as data so the message can name it — a user who
/// passed `--config /typo.toml` needs to see *that* file, not the
/// generic `jinn.toml` a default-path failure would report.
#[derive(Debug, Error)]
#[error(debug)]
pub struct ConfigPathError {
    /// The `--config` path that could not be used.
    pub path: PathBuf,
}

/// Where the run's `jinn.toml` lives, and what still needs doing to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPathResolution {
    /// The document to read and write for the whole run.
    pub path: PathBuf,
    /// True only when `path` is the default location and did not exist.
    ///
    /// An override never sets this: the user supplies their own file, and
    /// a missing one is an error rather than something to create.
    pub seed_template: bool,
}

/// Chooses the `jinn.toml` for a run.
///
/// `override_path` is the `--config` value, if the user gave one.
/// `default_path` is the platform's per-user location.
///
/// # Errors
///
/// Returns [`ConfigPathError`] when `override_path` is given but does not
/// exist. The default path never errors — a missing one resolves to a
/// resolution with `seed_template: true`.
pub fn resolve_config_path(
    override_path: Option<&Path>,
    default_path: &Path,
) -> Result<ConfigPathResolution, Report<ConfigPathError>> {
    match override_path {
        Some(path) => resolve_override(path),
        None => Ok(resolve_default(default_path)),
    }
}

/// Resolves an explicit `--config` path, which must already exist.
fn resolve_override(path: &Path) -> Result<ConfigPathResolution, Report<ConfigPathError>> {
    if path.exists() {
        return Ok(ConfigPathResolution {
            path: path.to_path_buf(),
            seed_template: false,
        });
    }
    Err(Report::new(ConfigPathError {
        path: path.to_path_buf(),
    })
    .attach(format!("config file not found: {}", path.display()))
    .attach("create it first, or omit --config to use the default location"))
}

/// Resolves the platform default, flagging a missing file for seeding.
fn resolve_default(default_path: &Path) -> ConfigPathResolution {
    ConfigPathResolution {
        path: default_path.to_path_buf(),
        seed_template: !default_path.exists(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default location for a test: a `jinn.toml` that does not exist.
    fn absent_default() -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().expect("temp dir");
        let path = root.path().join("jinn.toml");
        (root, path)
    }

    #[rstest::rstest]
    #[test]
    fn absent_default_file_is_flagged_for_seeding() {
        // Given a default path that does not exist.
        let (_root, default) = absent_default();

        // When resolving.
        let result = resolve_config_path(None, &default).expect("resolves");

        // Then the default path is chosen.
        assert_eq!(result.path, default);
        // And it is flagged for template seeding.
        assert!(result.seed_template);
    }

    #[rstest::rstest]
    #[test]
    fn present_default_file_is_not_flagged_for_seeding() {
        // Given a default path that already exists.
        let (_root, default) = absent_default();
        std::fs::write(&default, "# existing\n").expect("write");

        // When resolving.
        let result = resolve_config_path(None, &default).expect("resolves");

        // Then the default path is chosen and nothing needs seeding.
        assert_eq!(result.path, default);
        assert!(!result.seed_template);
    }

    #[rstest::rstest]
    #[test]
    fn existing_override_is_chosen() {
        // Given a default path and a separate override file that exists.
        let (_root, default) = absent_default();
        let override_root = tempfile::tempdir().expect("temp dir");
        let override_path = override_root.path().join("alt.toml");
        std::fs::write(&override_path, "# alt\n").expect("write");

        // When resolving.
        let result = resolve_config_path(Some(&override_path), &default).expect("resolves");

        // Then the override path is chosen.
        assert_eq!(result.path, override_path);
        // And it is not flagged for seeding — the user supplied the file.
        assert!(!result.seed_template);
    }

    #[rstest::rstest]
    #[test]
    fn missing_override_is_rejected() {
        // Given an override path that does not exist.
        let (_root, default) = absent_default();
        let override_root = tempfile::tempdir().expect("temp dir");
        let override_path = override_root.path().join("missing.toml");

        // When resolving.
        let result = resolve_config_path(Some(&override_path), &default);

        // Then resolution fails.
        let report = result.expect_err("missing override must fail");
        // And the error names the offending path.
        assert_eq!(report.current_context().path, override_path);
        // And nothing was created on disk.
        assert!(!override_path.exists());
    }

    #[rstest::rstest]
    #[test]
    fn missing_override_ignores_a_missing_default() {
        // Given a missing override alongside a default that also does not exist.
        let (_root, default) = absent_default();
        let override_root = tempfile::tempdir().expect("temp dir");
        let override_path = override_root.path().join("missing.toml");

        // When resolving.
        let result = resolve_config_path(Some(&override_path), &default);

        // Then it fails on the override rather than seeding the default.
        assert!(result.is_err());
        // And the default was not seeded.
        assert!(!default.exists());
    }
}
