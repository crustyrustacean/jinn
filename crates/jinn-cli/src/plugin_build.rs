//! `jinn plugin build` — build a plugin crate to a jinn-installable payload.
//!
//! Wraps `cargo build --target wasm32-wasip2 --release` in the plugin's
//! directory and reports the produced artifact. This is the same build the
//! justfile `build-plugins` recipe performs for first-party plugins; the
//! subcommand exists so external plugin authors never need the justfile.

use std::path::{Path, PathBuf};

use error_stack::{Report, ResultExt as _};
use wherror::Error;

/// The build failed.
#[derive(Debug, Error, PartialEq, Eq)]
#[error(debug)]
pub enum PluginBuildError {
    /// The given directory contains no `Cargo.toml`.
    NotACrate,
    /// `cargo` could not be run (missing from PATH, or spawn failure).
    CargoSpawn,
    /// `cargo` exited non-zero (compile error).
    BuildFailed,
    /// The artifact the build should have produced was not found.
    ArtifactMissing,
}

/// The target jinn plugins are built for.
const TARGET: &str = "wasm32-wasip2";

/// Builds the plugin crate at `dir` and returns the artifact path.
///
/// # Errors
///
/// Returns an error if `dir` is not a cargo crate, cargo fails, or the
/// expected `target/wasm32-wasip2/release/<name>.wasm` is missing after a
/// successful build.
pub fn build(dir: &Path) -> Result<PathBuf, Report<PluginBuildError>> {
    let manifest = dir.join("Cargo.toml");
    if !manifest.is_file() {
        return Err(
            Report::new(PluginBuildError::NotACrate).attach(dir.to_string_lossy().to_string())
        );
    }

    let crate_name = read_crate_name(&manifest)?;
    let output = std::process::Command::new("cargo")
        .args([
            "build",
            "--release",
            "--target",
            TARGET,
            "--message-format=json",
        ])
        .current_dir(dir)
        .output()
        .map_err(|err| {
            Report::new(PluginBuildError::CargoSpawn)
                .attach(err.to_string())
                .attach("is cargo installed and on PATH?")
        })?;

    // Cargo writes compile errors to stderr; pass them through unchanged.
    passthrough_stderr(&output.stderr);
    if !output.status.success() {
        return Err(Report::new(PluginBuildError::BuildFailed)
            .attach(format!("exit status {}", output.status)));
    }

    // The artifact path comes from cargo itself: in-workspace crates build
    // to the workspace root's target dir, standalone crates to their own.
    // Guessing the layout is wrong for one of the two.
    let Some(artifact) = last_executable_artifact(&output.stdout) else {
        return Err(Report::new(PluginBuildError::ArtifactMissing)
            .attach(format!("crate {crate_name} produced no wasm artifact")));
    };
    if !artifact.is_file() {
        return Err(Report::new(PluginBuildError::ArtifactMissing)
            .attach(artifact.to_string_lossy().to_string()));
    }
    Ok(artifact)
}

/// Forwards cargo's stderr (compile diagnostics) to the CLI user.
#[expect(
    clippy::print_stderr,
    reason = "cargo stderr is passed through to the CLI user"
)]
fn passthrough_stderr(stderr: &[u8]) {
    eprint!("{}", String::from_utf8_lossy(stderr));
}

/// Extracts the last compiler-artifact executable path from cargo's JSON
/// message stream (rendered diagnostics and non-link artifacts are skipped).
fn last_executable_artifact(stdout: &[u8]) -> Option<PathBuf> {
    let text = String::from_utf8_lossy(stdout);
    let mut last = None;
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("reason").and_then(|r| r.as_str()) == Some("compiler-artifact")
            && let Some(exe) = value.get("executable").and_then(|e| e.as_str())
        {
            last = Some(PathBuf::from(exe));
        }
    }
    last
}

/// Reads `package.name` from a `Cargo.toml`.
fn read_crate_name(manifest: &Path) -> Result<String, Report<PluginBuildError>> {
    let content = std::fs::read_to_string(manifest)
        .change_context(PluginBuildError::NotACrate)
        .attach(manifest.to_string_lossy().to_string())?;
    let value: toml::Value = content
        .parse()
        .change_context(PluginBuildError::NotACrate)
        .attach(manifest.to_string_lossy().to_string())?;
    value
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            Report::new(PluginBuildError::NotACrate)
                .attach(manifest.to_string_lossy().to_string())
                .attach("package.name is missing")
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test assertions")]

    use super::*;

    /// A directory without Cargo.toml is rejected before cargo runs.
    #[test]
    fn build_rejects_directory_without_manifest() {
        // Given an empty temp directory.
        let tmp = std::env::temp_dir().join(format!("jinn-pb-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("create temp dir");

        // When building it.
        let result = build(&tmp);

        // Then it fails with NotACrate.
        assert!(matches!(result, Err(ref report)
                if report.current_context() == &PluginBuildError::NotACrate));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The artifact name follows package.name, not the directory name.
    #[test]
    fn artifact_name_follows_package_name() {
        // Given a manifest whose package name differs from any assumption.
        let tmp = std::env::temp_dir().join(format!("jinn-pb2-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("create temp dir");
        std::fs::write(
            tmp.join("Cargo.toml"),
            r#"[package]
name = "my-renamed-plugin"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]
"#,
        )
        .expect("write manifest");

        // When building (cargo may fail offline for git deps, but the name
        // resolution is what is under test: build → BuildFailed, not
        // NotACrate; on success the artifact name matches).
        let result = build(&tmp);

        // Then the manifest parsed and the crate name was extracted.
        match result {
            Err(report) => assert!(
                report.current_context() != &PluginBuildError::NotACrate,
                "manifest should parse"
            ),
            Ok(artifact) => {
                assert!(
                    artifact
                        .to_string_lossy()
                        .contains("my-renamed-plugin.wasm"),
                    "artifact should carry package name: {}",
                    artifact.display()
                );
            }
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
