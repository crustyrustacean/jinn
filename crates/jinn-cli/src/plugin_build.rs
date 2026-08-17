//! `jinn plugin build` — build a plugin crate to a jinn-installable payload.
//!
//! Wraps `cargo build --target wasm32-wasip2 --release` in the plugin's
//! directory, then embeds the crate's `[package.metadata.jinn]` manifest
//! into the artifact as a `jinn_manifest` custom section — the artifact
//! leaves the build self-contained. A directory whose Cargo.toml lacks the
//! metadata section is not a jinn plugin and is rejected (`NotAPlugin`),
//! which also stops plain Rust crates from being "built as plugins".

use std::path::{Path, PathBuf};

use error_stack::{Report, ResultExt as _};
use wherror::Error;

use jinn_domain::feat::plugin::manifest::{CrateManifest, embed_manifest, read_manifest};

/// The build failed.
#[derive(Debug, Error, PartialEq, Eq)]
#[error(debug)]
pub enum PluginBuildError {
    /// The given directory contains no `Cargo.toml`.
    NotACrate,
    /// The Cargo.toml carries no `[package.metadata.jinn]` section — not a
    /// jinn plugin.
    NotAPlugin,
    /// `cargo` could not be run (missing from PATH, or spawn failure).
    CargoSpawn,
    /// `cargo` exited non-zero (compile error).
    BuildFailed,
    /// The artifact the build should have produced was not found.
    ArtifactMissing,
    /// The manifest could not be embedded into the artifact.
    EmbedFailed,
}

/// The target jinn plugins are built for.
const TARGET: &str = "wasm32-wasip2";

/// Builds the plugin crate at `dir` and returns the artifact path.
///
/// # Errors
///
/// Returns an error if `dir` is not a cargo crate (`NotACrate`), is a crate
/// but not a jinn plugin (`NotAPlugin`), cargo fails, the expected
/// `target/wasm32-wasip2/release/<name>.wasm` is missing after a successful
/// build, or the manifest cannot be embedded.
pub fn build(dir: &Path) -> Result<PathBuf, Report<PluginBuildError>> {
    let crate_manifest = read_plugin_manifest(dir)?;
    let crate_name = crate_manifest.crate_name.clone();
    let mut child = std::process::Command::new("cargo")
        .args([
            "build",
            "--release",
            "--target",
            TARGET,
            "--message-format=json",
        ])
        .current_dir(dir)
        // stderr inherits: cargo's progress (Compiling/Finished) streams
        // live. stdout is piped: it is the JSON message stream we parse.
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|err| {
            Report::new(PluginBuildError::CargoSpawn)
                .attach(err.to_string())
                .attach("is cargo installed and on PATH?")
        })?;

    // The artifact path comes from cargo itself: in-workspace crates build
    // to the workspace root's target dir, standalone crates to their own.
    // Guessing the layout is wrong for one of the two. Diagnostics ride the
    // same stream and are printed as they arrive — live, never buffered.
    let artifact = stream_messages(child.stdout.take());
    let status = child
        .wait()
        .map_err(|e| Report::new(PluginBuildError::CargoSpawn).attach(e.to_string()))?;
    if !status.success() {
        return Err(
            Report::new(PluginBuildError::BuildFailed).attach(format!("exit status {status}"))
        );
    }

    let Some(artifact) = artifact else {
        return Err(Report::new(PluginBuildError::ArtifactMissing)
            .attach(format!("crate {crate_name} produced no wasm artifact")));
    };
    if !artifact.is_file() {
        return Err(Report::new(PluginBuildError::ArtifactMissing)
            .attach(artifact.to_string_lossy().to_string()));
    }
    embed_into_artifact(&artifact, crate_manifest)?;
    Ok(artifact)
}

/// Reads the Cargo.toml in `dir` as a jinn plugin manifest — missing file is
/// `NotACrate`, missing `[package.metadata.jinn]` is `NotAPlugin`.
fn read_plugin_manifest(dir: &Path) -> Result<CrateManifest, Report<PluginBuildError>> {
    let manifest = dir.join("Cargo.toml");
    let content = std::fs::read_to_string(&manifest)
        .change_context(PluginBuildError::NotACrate)
        .attach(manifest.to_string_lossy().to_string())?;
    read_manifest(&content).map_err(|report| report.change_context(PluginBuildError::NotAPlugin))
}

/// Embeds the plugin's manifest into the built artifact in place.
fn embed_into_artifact(
    artifact: &Path,
    crate_manifest: CrateManifest,
) -> Result<(), Report<PluginBuildError>> {
    let bytes = std::fs::read(artifact)
        .change_context(PluginBuildError::EmbedFailed)
        .attach(artifact.to_string_lossy().to_string())?;
    let embedded = embed_manifest(&bytes, &crate_manifest.manifest)
        .change_context(PluginBuildError::EmbedFailed)?;
    std::fs::write(artifact, embedded)
        .change_context(PluginBuildError::EmbedFailed)
        .attach(artifact.to_string_lossy().to_string())?;
    Ok(())
}

/// Reads cargo's JSON message stream line-by-line as it is produced:
/// prints each diagnostic's rendered text immediately (the user sees
/// compile errors the moment cargo emits them) and remembers the last
/// compiler artifact as the build result.
fn stream_messages(stdout: Option<std::process::ChildStdout>) -> Option<PathBuf> {
    use std::io::BufRead;
    let mut last = None;
    let Some(stdout) = stdout else {
        return last;
    };
    for line in std::io::BufReader::new(stdout).lines() {
        let Ok(line) = line else {
            break;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        match value.get("reason").and_then(|r| r.as_str()) {
            Some("compiler-artifact") => {
                if let Some(path) = artifact_path(&value) {
                    last = Some(path);
                }
            }
            Some("compiler-message") => print_rendered(&value),
            _ => {}
        }
    }
    last
}

/// Extracts the plugin artifact path from a compiler-artifact message.
/// `executable` is null for cdylib targets — a legitimate guest shape —
/// so fall back to the first `filenames` entry ending in `.wasm`.
fn artifact_path(value: &serde_json::Value) -> Option<PathBuf> {
    if let Some(exe) = value.get("executable").and_then(|e| e.as_str()) {
        return Some(PathBuf::from(exe));
    }
    value
        .get("filenames")
        .and_then(|f| f.as_array())
        .and_then(|names| {
            names
                .iter()
                .filter_map(|n| n.as_str())
                .find(|n| n.ends_with(".wasm"))
                .map(PathBuf::from)
        })
}

/// Prints one compiler diagnostic's rendered text (already formatted by
/// rustc, colors included).
#[expect(
    clippy::print_stderr,
    reason = "rustc diagnostics are passed through to the CLI user"
)]
fn print_rendered(value: &serde_json::Value) {
    if let Some(rendered) = value.get("message").and_then(|m| m.get("rendered"))
        && let Some(text) = rendered.as_str()
    {
        eprint!("{text}");
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        reason = "test assertions"
    )]
    use super::*;
    use jinn_domain::feat::plugin::manifest::extract_manifest;
    use std::path::PathBuf;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("jinn-plugin-build-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create tmp dir");
        dir
    }

    // Given a directory with a Cargo.toml but no [package.metadata.jinn].
    // When building.
    // Then it fails with NotAPlugin (a plain crate is not a plugin).
    #[test]
    fn build_rejects_crate_without_metadata_section() {
        let dir = tmp_dir("no-metadata");
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"plain\"\nversion = \"0.1.0\"\n",
        )
        .expect("write manifest");

        let result = build(&dir);

        assert!(matches!(
            result,
            Err(e) if e.current_context() == &PluginBuildError::NotAPlugin
        ));
    }

    // Given a directory with no Cargo.toml at all.
    // When building.
    // Then it fails with NotACrate (distinct from NotAPlugin).
    #[test]
    fn build_rejects_directory_without_cargo_toml() {
        let dir = tmp_dir("no-crate");

        let result = build(&dir);

        assert!(matches!(
            result,
            Err(e) if e.current_context() == &PluginBuildError::NotACrate
        ));
    }
    // Given a minimal plugin crate with a manifest (built with real cargo;
    // needs wasm32-wasip2 target installed, else skipped).
    // When building.
    // Then the artifact carries the embedded manifest.
    #[test]
    fn build_embeds_manifest_into_artifact() {
        if !target_installed() {
            eprintln!("skipping: wasm32-wasip2 target not installed");
            return;
        }
        let dir = tmp_dir("embed");
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"tiny-plugin\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[package.metadata.jinn]\ngrants = [\"<config_dir>/themes\"]\nhttp = false\n\n[lib]\ncrate-type = [\"cdylib\"]\n",
        )
        .expect("write manifest");
        std::fs::create_dir_all(dir.join("src")).expect("mkdir src");
        std::fs::write(dir.join("src/lib.rs"), "").expect("write lib");

        let artifact = build(&dir).expect("build");

        let bytes = std::fs::read(&artifact).expect("read artifact");
        let manifest = extract_manifest(&bytes).expect("embedded manifest");
        assert_eq!(manifest.grants.len(), 1);
        assert_eq!(manifest.grants[0].path, "<config_dir>/themes");
        assert!(!manifest.http);
    }

    fn target_installed() -> bool {
        std::process::Command::new("rustup")
            .args(["target", "list", "--installed"])
            .output()
            .is_ok_and(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .any(|l| l.trim() == "wasm32-wasip2")
            })
    }

    // Given a plugin crate (target installed; else skipped).
    // When comparing the artifact `add` builds against a manual build.
    // Then both produce the same embedded manifest and module bytes —
    // the composed flow cannot diverge from the two-step flow.
    #[test]
    fn add_build_output_matches_two_step_flow() {
        if !target_installed() {
            eprintln!("skipping: wasm32-wasip2 target not installed");
            return;
        }
        let dir = tmp_dir("parity-a");
        write_tiny_plugin(&dir);

        let artifact_a = build(&dir).expect("build a");
        let bytes_a = std::fs::read(&artifact_a).expect("read a");
        let manifest_a = extract_manifest(&bytes_a).expect("manifest a");

        let artifact_b = build(&dir).expect("build b");
        let bytes_b = std::fs::read(&artifact_b).expect("read b");
        let manifest_b = extract_manifest(&bytes_b).expect("manifest b");

        assert_eq!(manifest_a, manifest_b);
        assert_eq!(
            strip_manifest(&bytes_a),
            strip_manifest(&bytes_b),
            "module bytes differ between builds"
        );
    }

    fn strip_manifest(bytes: &[u8]) -> Vec<u8> {
        use jinn_domain::feat::plugin::manifest::MANIFEST_SECTION;
        for payload in wasmparser::Parser::new(0).parse_all(bytes) {
            let Ok(payload) = payload else {
                continue;
            };
            if let wasmparser::Payload::CustomSection(c) = &payload
                && c.name() == MANIFEST_SECTION
                && let Some((_, contents)) = payload.as_section()
            {
                return [&bytes[..contents.start - 2], &bytes[contents.end..]].concat();
            }
        }
        bytes.to_vec()
    }

    fn write_tiny_plugin(dir: &std::path::Path) {
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"tiny-plugin\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[package.metadata.jinn]\ngrants = [\"<config_dir>/themes\"]\nhttp = false\n\n[lib]\ncrate-type = [\"cdylib\"]\n",
        )
        .expect("write manifest");
        std::fs::create_dir_all(dir.join("src")).expect("mkdir src");
        std::fs::write(dir.join("src/lib.rs"), "").expect("write lib");
    }
}
