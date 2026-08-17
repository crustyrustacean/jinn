//! `jinn plugin new` — scaffold a new wasm plugin cargo project.
//!
//! Writes a minimal but complete plugin into `<name>/` in the current
//! directory: a `Cargo.toml` pinned to the SDK, a `main.rs` performing the
//! wire handshake and one placeholder contribution, and a `.gitignore`.
//! The scaffold compiles as-is; building it with
//! `cargo build --target wasm32-wasip2 --release` produces the `.wasm`
//! payload `jinn plugin install` consumes.

use std::path::Path;

use error_stack::{Report, ResultExt as _};
use wherror::Error;

/// The scaffold failed to write.
#[derive(Debug, Error, PartialEq, Eq)]
#[error(debug)]
pub enum PluginNewError {
    /// The target directory already exists.
    Exists,
    /// A file could not be written.
    Write,
    /// The name is not a valid crate name.
    InvalidName,
}

/// Crate-name validation: lowercase ASCII alphanumerics and dashes.
fn valid_crate_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The `main.rs` template with the plugin name substituted.
fn main_rs(name: &str) -> String {
    format!(
        r#"//! {name} — a jinn wasm plugin.
//!
//! Wire contract: `Hello` → await `Welcome` → push contributions → exit.
//! The host keeps whatever you pushed cached after your process ends, so
//! a push-once plugin that exits is a complete, correct plugin.

use jinn_plugin_api::{{PluginToHost, SetThemeEntries}};
use jinn_plugin_sdk::{{PluginOutput, hello, push, welcome}};

fn main() {{
    let mut out = PluginOutput::stdout();
    if hello(&mut out, "{name}").is_err() {{
        return;
    }}
    let Ok(grants) = welcome() else {{
        return;
    }};
    // `grants.read_dirs` are the directories the manifest granted you;
    // `grants.write_dirs` your scratch space; `grants.http` the network flag.

    // TODO: replace this placeholder contribution with your plugin's data.
    let _ = &grants;
    let _ = push(
        &mut out,
        PluginToHost::SetThemeEntries(SetThemeEntries {{
            themes: vec![],
        }}),
    );
}}
"#
    )
}

/// The `Cargo.toml` template. Dependencies point at the jinn repo (git):
/// the SDK crates are not published to crates.io.
const CARGO_TOML: &str = r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

[dependencies]
jinn-plugin-api = { git = "https://github.com/jayson-lennon/jinn" }
jinn-plugin-sdk = { git = "https://github.com/jayson-lennon/jinn" }
serde_json = "1"

[[bin]]
name = "{name}"
path = "src/main.rs"
"#;

/// The `.gitignore` template.
const GITIGNORE: &str = "/target\n";

/// The post-scaffold instructions.
const INSTRUCTIONS: &str = r#"Next steps:

  cd {name}
  rustup target add wasm32-wasip2        # once per toolchain
  jinn plugin build                      # builds + prints the artifact path
  jinn plugin install <printed-path>
  # restart jinn — plugins activate at startup

Dependencies resolve from the jinn git repo (first build clones it).
Edit src/main.rs to push real data over the wire. See the jinn-plugin
SKILL.md (installed to your skills dir) for the full authoring loop.
"#;

/// Scaffolds the plugin project at `base/<name>`.
///
/// # Errors
///
/// Returns an error if the name is invalid, the directory exists, or any
/// file write fails.
pub fn scaffold(base: &Path, name: &str) -> Result<std::path::PathBuf, Report<PluginNewError>> {
    if !valid_crate_name(name) {
        return Err(Report::new(PluginNewError::InvalidName)
            .attach(format!("name: {name}"))
            .attach("use lowercase ascii, digits, and dashes"));
    }
    let dir = base.join(name);
    if dir.exists() {
        return Err(Report::new(PluginNewError::Exists).attach(dir.to_string_lossy().to_string()));
    }
    let src = dir.join("src");
    std::fs::create_dir_all(&src)
        .change_context(PluginNewError::Write)
        .attach(dir.to_string_lossy().to_string())?;

    for (path, contents) in [
        (dir.join("Cargo.toml"), CARGO_TOML.replace("{name}", name)),
        (src.join("main.rs"), main_rs(name)),
        (dir.join(".gitignore"), GITIGNORE.to_owned()),
    ] {
        std::fs::write(&path, contents)
            .change_context(PluginNewError::Write)
            .attach(path.to_string_lossy().to_string())?;
    }
    Ok(dir)
}

/// Prints the scaffold outcome to stdout.
#[expect(clippy::print_stdout, reason = "CLI user-facing output path")]
pub fn report_success(dir: &Path, name: &str) {
    println!("Scaffolded plugin at {}", dir.display());
    print!("{}", INSTRUCTIONS.replace("{name}", name));
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

    // Given a lowercase-dash name.
    // Then it validates.
    #[test]
    fn valid_name_accepted() {
        assert!(valid_crate_name("my-plugin"));
    }

    // Given a name with uppercase or underscores.
    // Then it is rejected.
    #[rstest::rstest]
    #[case("")]
    #[case("MyPlugin")]
    #[case("my_plugin")]
    #[case("my plugin")]
    fn invalid_names_rejected(#[case] name: &str) {
        assert!(!valid_crate_name(name));
    }

    // Given an empty base directory.
    // When scaffolding "probe".
    // Then the full project layout exists with the name substituted.
    #[test]
    fn scaffold_writes_project_layout() {
        // Given an empty temp base.
        let base = std::env::temp_dir().join(format!("jinn-plugin-new-{}", std::process::id()));
        std::fs::create_dir_all(&base).expect("base");

        // When scaffolding.
        let dir = scaffold(&base, "probe").expect("scaffold");

        // Then the layout exists.
        assert!(dir.join("Cargo.toml").is_file());
        assert!(dir.join("src/main.rs").is_file());
        assert!(dir.join(".gitignore").is_file());
        let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).expect("read");
        assert!(manifest.contains("name = \"probe\""));
        // And dependencies resolve from the jinn git repo, not crates.io.
        assert!(manifest.contains("git = \"https://github.com/jayson-lennon/jinn\""));
        let main = std::fs::read_to_string(dir.join("src/main.rs")).expect("read");
        assert!(main.contains("\"probe\""));

        let _ = std::fs::remove_dir_all(&base);
    }

    // Given an existing directory with the plugin's name.
    // When scaffolding.
    // Then it fails with Exists and writes nothing.
    #[test]
    fn scaffold_refuses_existing_directory() {
        // Given a base with an existing "probe" dir.
        let base = std::env::temp_dir().join(format!("jinn-plugin-new-e-{}", std::process::id()));
        std::fs::create_dir_all(base.join("probe")).expect("base");

        // When scaffolding.
        let result = scaffold(&base, "probe");

        // Then it fails with Exists.
        let Err(report) = result else {
            panic!("expected Exists");
        };
        assert_eq!(report.current_context(), &PluginNewError::Exists);

        let _ = std::fs::remove_dir_all(&base);
    }

    // Given an invalid name.
    // When scaffolding.
    // Then it fails with InvalidName.
    #[test]
    fn scaffold_rejects_invalid_name() {
        // Given any base.
        let base = std::env::temp_dir();

        // When scaffolding with "Not A Name".
        let result = scaffold(&base, "Not A Name");

        // Then it fails with InvalidName.
        let Err(report) = result else {
            panic!("expected InvalidName");
        };
        assert_eq!(report.current_context(), &PluginNewError::InvalidName);
    }
}
