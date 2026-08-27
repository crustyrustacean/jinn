//! `jinn plugin install` — register a built `.wasm` with jinn.
//!
//! Copies the payload into jinn's plugin directory and writes the
//! `[plugin.<name>]` entry into `jinn.toml` via the user-preferences storage
//! (comment-preserving patch). Grants and http come from the caller — the CLI
//! resolves manifest vs flag precedence and passes the effective values in.
//! Installation takes effect on the next jinn start: plugins spawn at app
//! boot.

use std::path::{Path, PathBuf};

use error_stack::{Report, ResultExt as _};
use wherror::Error;

use crate::feat::plugin::PluginConfig;

/// The install failed.
#[derive(Debug, Error, PartialEq, Eq)]
#[error(debug)]
pub enum PluginInstallError {
    /// The wasm path does not exist.
    MissingWasm,
    /// The payload is not a `.wasm` file.
    NotWasm,
    /// The plugins directory could not be created.
    CreateDir,
    /// The copy failed.
    Copy,
    /// The `[[plugin]]` entry could not be written to `jinn.toml`.
    WriteConfig,
    /// The plugin name is not a valid crate-style name.
    InvalidName,
}

/// Outcome of a successful install.
#[derive(Debug, PartialEq, Eq)]
pub enum PluginInstallOutcome {
    /// Freshly installed.
    Installed {
        /// Destination of the copied payload.
        wasm_path: PathBuf,
        /// The entry name written to `jinn.toml`.
        name: String,
    },
    /// An entry with this name already existed and was updated.
    Updated {
        /// Destination of the copied payload.
        wasm_path: PathBuf,
        /// The entry name written to `jinn.toml`.
        name: String,
    },
}

/// Crate-name validation shared with the scaffold.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Installs `wasm_path` as plugin `name`.
///
/// Copies the payload to `plugins_dir` and appends/updates the `[[plugin]]`
/// entry via the given storage. Does **not** spawn anything — the restart
/// does.
///
/// # Errors
///
/// Returns an error if the payload is missing, the directory cannot be
/// created, the copy fails, or the config write fails.
pub fn install(
    wasm_path: &Path,
    name: &str,
    plugins_dir: &Path,
    grants: Vec<crate::feat::plugin::PluginPathGrant>,
    http: bool,
    storage: &dyn crate::feat::preferences_actor::user_preferences_storage::UserPreferencesStorage,
) -> Result<PluginInstallOutcome, Report<PluginInstallError>> {
    if !valid_name(name) {
        return Err(Report::new(PluginInstallError::InvalidName).attach(format!("name: {name}")));
    }
    if !wasm_path.is_file() {
        return Err(Report::new(PluginInstallError::MissingWasm)
            .attach(wasm_path.to_string_lossy().to_string()));
    }
    if wasm_path.extension().is_none_or(|e| e != "wasm") {
        return Err(Report::new(PluginInstallError::NotWasm)
            .attach(wasm_path.to_string_lossy().to_string()));
    }

    std::fs::create_dir_all(plugins_dir)
        .change_context(PluginInstallError::CreateDir)
        .attach(plugins_dir.to_string_lossy().to_string())?;

    let dest = plugins_dir.join(format!("{name}.wasm"));
    std::fs::copy(wasm_path, &dest)
        .change_context(PluginInstallError::Copy)
        .attach(format!(
            "copying {} to {}",
            wasm_path.display(),
            dest.display()
        ))?;

    let existed = register_entry(name, grants, http, storage)?;

    Ok(if existed {
        PluginInstallOutcome::Updated {
            wasm_path: dest,
            name: name.to_owned(),
        }
    } else {
        PluginInstallOutcome::Installed {
            wasm_path: dest,
            name: name.to_owned(),
        }
    })
}

/// Writes the `[plugin.<name>]` entry into preferences storage, returning
/// whether an entry with this name already existed.
///
/// Shared by `jinn plugin install` (flag/manifest-resolved values) and the
/// bundled first-party plugin seeding (`jinn install`, manifest-resolved
/// values). The entry is always left `enabled`.
///
/// # Errors
///
/// Returns [`Report<PluginInstallError::WriteConfig>`] if preferences
/// cannot be reloaded or saved.
pub fn register_plugin(
    name: &str,
    manifest: &crate::feat::plugin::manifest::PluginManifest,
    storage: &dyn crate::feat::preferences_actor::user_preferences_storage::UserPreferencesStorage,
) -> Result<bool, Report<PluginInstallError>> {
    register_entry(name, manifest.grants.clone(), manifest.http, storage)
}

fn register_entry(
    name: &str,
    grants: Vec<crate::feat::plugin::PluginPathGrant>,
    http: bool,
    storage: &dyn crate::feat::preferences_actor::user_preferences_storage::UserPreferencesStorage,
) -> Result<bool, Report<PluginInstallError>> {
    let entry = PluginConfig {
        wasm: format!("{name}.wasm"),
        grants,
        http,
        config: None,
        enabled: true,
    };

    let mut prefs = storage
        .reload()
        .change_context(PluginInstallError::WriteConfig)?;
    let existed = prefs.plugin.remove(name).is_some();
    prefs.plugin.insert(name.to_owned(), entry);
    storage
        .save(&prefs)
        .change_context(PluginInstallError::WriteConfig)?;
    Ok(existed)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::print_stdout,
        clippy::print_stderr,
        reason = "test assertions"
    )]
    #![expect(clippy::let_underscore_must_use, reason = "none used")]

    use super::*;
    use crate::feat::preferences_actor::user_preferences_storage::{
        InMemoryUserPreferencesStorage, UserPreferencesStorage as _,
    };

    // Given a temp wasm file and in-memory storage.
    // When installing.
    // Then the payload is copied, the entry appended, and Installed returned.
    #[rstest::rstest]
    #[test]
    fn install_copies_payload_and_appends_entry() {
        // Given a temp wasm payload.
        let base = std::env::temp_dir().join(format!("jinn-pi-{}", std::process::id()));
        std::fs::create_dir_all(&base).expect("base");
        let wasm = base.join("my-plugin.wasm");
        std::fs::write(&wasm, b"mock wasm").expect("write");

        // When installing.
        let storage = InMemoryUserPreferencesStorage::default();
        let outcome = install(
            &wasm,
            "my-plugin",
            &base.join("plugins"),
            vec![],
            false,
            &storage,
        )
        .expect("install");

        // Then the payload was copied and the entry appended.
        assert!(base.join("plugins/my-plugin.wasm").is_file());
        let prefs = storage.reload().expect("reload");
        assert!(
            prefs
                .plugin
                .get("my-plugin")
                .is_some_and(|p| p.wasm == "my-plugin.wasm")
        );
        assert!(matches!(
            outcome,
            PluginInstallOutcome::Installed { ref name, .. } if name == "my-plugin"
        ));

        let _ = std::fs::remove_dir_all(&base);
    }

    // Given an existing [[plugin]] entry with the same name.
    // When installing again.
    // Then the entry is replaced (not duplicated) and Updated returned.
    #[rstest::rstest]
    #[test]
    fn install_replaces_existing_entry() {
        // Given a prior install.
        let base = std::env::temp_dir().join(format!("jinn-pi-r-{}", std::process::id()));
        std::fs::create_dir_all(&base).expect("base");
        let wasm = base.join("my-plugin.wasm");
        std::fs::write(&wasm, b"v1").expect("write");
        let storage = InMemoryUserPreferencesStorage::default();
        install(
            &wasm,
            "my-plugin",
            &base.join("plugins"),
            vec![],
            false,
            &storage,
        )
        .expect("first");

        // When installing v2.
        std::fs::write(&wasm, b"v2 longer payload").expect("write v2");
        let outcome = install(
            &wasm,
            "my-plugin",
            &base.join("plugins"),
            vec![],
            false,
            &storage,
        )
        .expect("second");

        // Then exactly one entry exists and Updated was returned.
        let prefs = storage.reload().expect("reload");
        assert_eq!(prefs.plugin.keys().filter(|n| *n == "my-plugin").count(), 1);
        assert!(matches!(outcome, PluginInstallOutcome::Updated { .. }));

        let _ = std::fs::remove_dir_all(&base);
    }

    // Given a missing wasm path.
    // When installing.
    // Then it fails with MissingWasm.
    #[rstest::rstest]
    #[test]
    fn install_fails_when_wasm_missing() {
        // Given no file.
        let base = std::env::temp_dir().join(format!("jinn-pi-m-{}", std::process::id()));

        // When installing.
        let result = install(
            &base.join("nope.wasm"),
            "x",
            &base,
            vec![],
            false,
            &InMemoryUserPreferencesStorage::default(),
        );

        // Then MissingWasm.
        let Err(report) = result else {
            panic!("expected MissingWasm");
        };
        assert_eq!(report.current_context(), &PluginInstallError::MissingWasm);
    }

    // Given a non-wasm file.
    // When installing.
    // Then it fails with NotWasm.
    #[rstest::rstest]
    #[test]
    fn install_fails_for_non_wasm_file() {
        // Given a .txt file.
        let base = std::env::temp_dir().join(format!("jinn-pi-t-{}", std::process::id()));
        std::fs::create_dir_all(&base).expect("base");
        let txt = base.join("plugin.txt");
        std::fs::write(&txt, b"text").expect("write");

        // When installing.
        let result = install(
            &txt,
            "x",
            &base,
            vec![],
            false,
            &InMemoryUserPreferencesStorage::default(),
        );

        // Then NotWasm.
        let Err(report) = result else {
            panic!("expected NotWasm");
        };
        assert_eq!(report.current_context(), &PluginInstallError::NotWasm);

        let _ = std::fs::remove_dir_all(&base);
    }

    // Given an invalid name.
    // When installing.
    // Then it fails with InvalidName before touching the filesystem.
    #[rstest::rstest]
    #[test]
    fn install_rejects_invalid_name() {
        // Given a valid wasm but a bad name.
        let base = std::env::temp_dir().join(format!("jinn-pi-n-{}", std::process::id()));
        std::fs::create_dir_all(&base).expect("base");
        let wasm = base.join("p.wasm");
        std::fs::write(&wasm, b"w").expect("write");

        // When installing with an uppercase name.
        let result = install(
            &wasm,
            "BadName",
            &base,
            vec![],
            false,
            &InMemoryUserPreferencesStorage::default(),
        );

        // Then InvalidName.
        let Err(report) = result else {
            panic!("expected InvalidName");
        };
        assert_eq!(report.current_context(), &PluginInstallError::InvalidName);

        let _ = std::fs::remove_dir_all(&base);
    }
}
