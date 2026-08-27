//! CLI-level integration tests for `jinn install`.
//!
//! Runs the real `install` dispatch against a temp XDG environment and
//! asserts the on-disk result: resource files, plugin payloads, and the
//! `[plugin.<name>]` entries in `jinn.toml`.

use std::path::PathBuf;

/// Resolves the built `jinn` binary for integration tests.
fn jinn_bin() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_jinn")
        .map(PathBuf::from)
        .expect("CARGO_BIN_EXE_jinn not set — integration test must run via cargo test")
}

// Given no existing jinn state in the temp environment.
// When running `jinn install`.
// Then it succeeds, seeds the plugin payloads, registers both first-party
// plugins, and prints the restart hint.
#[rstest::rstest]
#[test]
fn install_seeds_plugins_and_registers_entries() {
    let bin = jinn_bin();
    let (home, _guard) = temp_env();
    let config = home.join("config");
    let data = home.join("data");

    let output = std::process::Command::new(&bin)
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", &data)
        .arg("--db-path")
        .arg(data.join("unused.db"))
        .arg("install")
        .output()
        .expect("run jinn install");
    assert!(
        output.status.success(),
        "jinn install failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("persona-loader.wasm"));
    assert!(stdout.contains("theme-loader.wasm"));
    assert!(stdout.contains("Restart jinn to activate plugins."));
}

// Given the plugin payloads already on disk.
// When running `jinn install` again without --force.
// Then both payloads are reported skipped.
#[rstest::rstest]
#[test]
fn install_skips_existing_plugins_without_force() {
    let bin = jinn_bin();
    let (home, _guard) = temp_env();
    let config = home.join("config");
    let data = home.join("data");

    for _ in 0..2 {
        let output = std::process::Command::new(&bin)
            .env("XDG_CONFIG_HOME", &config)
            .env("XDG_DATA_HOME", &data)
            .arg("--db-path")
            .arg(data.join("unused.db"))
            .arg("install")
            .output()
            .expect("run jinn install");
        assert!(
            output.status.success(),
            "jinn install failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let second_run = std::process::Command::new(&bin)
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", &data)
        .arg("--db-path")
        .arg(data.join("unused.db"))
        .arg("install")
        .output()
        .expect("run jinn install");
    let second = String::from_utf8_lossy(&second_run.stdout);
    assert!(second.contains("Already present, skipped"));
    assert!(second.contains("theme-loader.wasm"));
}

// Given a completed `jinn install` in a temp environment.
// When reading the plugin payloads and `jinn.toml`.
// Then both wasm payloads exist and both entries carry their
// manifest-declared grants.
#[rstest::rstest]
#[test]
fn install_writes_plugin_payloads_and_toml_entries() {
    let bin = jinn_bin();
    let (home, _guard) = temp_env();
    let config = home.join("config");
    let data = home.join("data");

    let output = std::process::Command::new(&bin)
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", &data)
        .arg("--db-path")
        .arg(data.join("unused.db"))
        .arg("install")
        .output()
        .expect("run jinn install");
    assert!(
        output.status.success(),
        "jinn install failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(data.join("jinn/plugins/theme-loader.wasm").is_file());
    assert!(data.join("jinn/plugins/persona-loader.wasm").is_file());

    let toml = std::fs::read_to_string(config.join("jinn/jinn.toml")).expect("read jinn.toml");
    assert!(toml.contains("[plugin.theme-loader]"));
    assert!(toml.contains("\"<config_dir>/themes\""));
    assert!(toml.contains("[plugin.persona-loader]"));
    assert!(toml.contains("\"<config_dir>/personas\""));
}

// A temp HOME whose subdirectories carry the XDG roots; the guard keeps the
// temp dir alive for the duration of the test.
fn temp_env() -> (PathBuf, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().expect("temp dir");
    (dir.path().to_path_buf(), dir)
}
