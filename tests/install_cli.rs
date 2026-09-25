//! CLI-level integration tests for `jinn install`.
//!
//! Runs the real `install` dispatch against a temp XDG environment and
//! asserts the on-disk result: resource files and the write-once
//! `jinn.toml` semantics (created only when absent; an existing file is
//! never modified, even with `--force`).

use std::path::PathBuf;

/// Resolves the built `jinn` binary for integration tests.
fn jinn_bin() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_jinn")
        .map(PathBuf::from)
        .expect("CARGO_BIN_EXE_jinn not set — integration test must run via cargo test")
}

/// Runs `jinn <args...>` inside the temp XDG environment.
fn run_jinn(
    bin: &std::path::Path,
    config: &std::path::Path,
    data: &std::path::Path,
    args: &[&str],
) -> std::process::Output {
    std::process::Command::new(bin)
        .env("XDG_CONFIG_HOME", config)
        .env("XDG_DATA_HOME", data)
        .arg("--db-path")
        .arg(data.join("unused.db"))
        .args(args)
        .output()
        .expect("run jinn")
}

// Given no existing jinn state in the temp environment.
// When running `jinn install`.
// Then it succeeds, seeds the four resource kinds (themes, personas,
// prompts, skills), lists jinn.toml as Created, and prints no plugin output.
#[rstest::rstest]
#[test]
fn install_seeds_all_resource_kinds_and_creates_jinn_toml() {
    let bin = jinn_bin();
    let (home, _guard) = temp_env();
    let config = home.join("config");
    let data = home.join("data");

    let output = run_jinn(&bin, &config, &data, &["install"]);
    assert!(
        output.status.success(),
        "jinn install failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("default.toml"), "theme seeded: {stdout}");
    assert!(stdout.contains("general.md"), "persona seeded: {stdout}");
    assert!(stdout.contains("plan.md"), "prompt seeded: {stdout}");
    assert!(stdout.contains("SKILL.md"), "skills seeded: {stdout}");
    assert!(
        stdout.contains("Created") && stdout.contains("jinn.toml"),
        "first install must list jinn.toml as Created: {stdout}"
    );
    // And no plugin artifacts or hints remain anywhere in the install output.
    assert!(!stdout.contains("plugin"), "no plugin output: {stdout}");
    assert!(!stdout.contains(".wasm"), "no wasm payloads: {stdout}");
}

// Given a completed first install.
// When running `jinn install` again without --force.
// Then jinn.toml is listed as skipped alongside the other resources.
#[rstest::rstest]
#[test]
fn install_second_run_lists_jinn_toml_as_skipped() {
    let bin = jinn_bin();
    let (home, _guard) = temp_env();
    let config = home.join("config");
    let data = home.join("data");

    let first = run_jinn(&bin, &config, &data, &["install"]);
    assert!(first.status.success());

    let second = run_jinn(&bin, &config, &data, &["install"]);
    assert!(second.status.success());
    let stdout = String::from_utf8_lossy(&second.stdout);
    let toml_line = stdout
        .lines()
        .find(|l| l.contains("jinn.toml"))
        .expect("output must mention jinn.toml");
    assert!(
        toml_line.starts_with("Already present, skipped"),
        "second run must list jinn.toml as skipped, got: {toml_line}"
    );
}

// Given a completed first install whose jinn.toml the user hand-edited
// (a compaction knob + a comment).
// When running `jinn install --force`.
// Then jinn.toml is byte-identical while resource files are overwritten.
#[rstest::rstest]
#[test]
fn install_force_preserves_edited_jinn_toml() {
    let bin = jinn_bin();
    let (home, _guard) = temp_env();
    let config = home.join("config");
    let data = home.join("data");

    let first = run_jinn(&bin, &config, &data, &["install"]);
    assert!(first.status.success());

    let toml_path = config.join("jinn/jinn.toml");
    let original = std::fs::read_to_string(&toml_path).expect("read jinn.toml");
    let edited = format!(
        "# user was here\n{}",
        original.replace("[compaction]", "[compaction]\nthreshold = 0.4",)
    );
    std::fs::write(&toml_path, &edited).expect("write edited jinn.toml");

    let forced = run_jinn(&bin, &config, &data, &["install", "--force"]);
    assert!(forced.status.success());
    let on_disk = std::fs::read_to_string(&toml_path).expect("read jinn.toml after --force");
    assert_eq!(on_disk, edited, "--force must never modify jinn.toml");
    // And the user's knob survived (the edit landed in the file).
    assert!(on_disk.contains("threshold = 0.4"));
    // And resource files still followed --force.
    let stdout = String::from_utf8_lossy(&forced.stdout);
    assert!(stdout.contains("Overwrote"));
}

// Given a malformed jinn.toml from a prior broken edit.
// When running `jinn install`.
// Then it succeeds and the file is left untouched.
#[rstest::rstest]
#[test]
fn install_succeeds_with_malformed_jinn_toml() {
    let bin = jinn_bin();
    let (home, _guard) = temp_env();
    let config = home.join("config");
    let data = home.join("data");

    let toml_dir = config.join("jinn");
    std::fs::create_dir_all(&toml_dir).expect("create config dir");
    std::fs::write(toml_dir.join("jinn.toml"), "NOT [valid toml").expect("write jinn.toml");

    let output = run_jinn(&bin, &config, &data, &["install"]);
    assert!(
        output.status.success(),
        "malformed jinn.toml must not fail install: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let on_disk = std::fs::read_to_string(toml_dir.join("jinn.toml")).expect("read jinn.toml");
    assert_eq!(
        on_disk, "NOT [valid toml",
        "install must not touch the file"
    );
}

// A temp HOME whose subdirectories carry the XDG roots; the guard keeps the
// temp dir alive for the duration of the test.
fn temp_env() -> (PathBuf, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().expect("temp dir");
    (dir.path().to_path_buf(), dir)
}
