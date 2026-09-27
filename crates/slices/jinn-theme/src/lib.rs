//! The theme slice — theme discovery and selection state.
//!
//! Owns one cell ([`theme_entries_slot`]) holding
//! [`jinn_theme_msg::ThemeEntries`]: the ordered theme selection the theme
//! picker displays and the app-state actor resolves the persisted theme
//! name against. `activate` scans the user and system theme directories
//! once — the same behavior the retired themes plugin had: one
//! bad file never drops the batch, and the user directory shadows the
//! system directory for same-name themes.

use std::path::Path;

use jinn_slices::SliceHost;
use jinn_theme::loader::{discover_themes, load_theme_from_file};
use jinn_theme_msg::NamedTheme;
use jinn_theme_msg::ThemeEntries;

pub(crate) mod theme_picker_actions;
pub mod theme_picker_render;
pub mod theme_picker_routes;
#[cfg(test)]
mod theme_picker_tests;
mod theme_picker_viewport;

pub use jinn_theme_msg::theme_entries_slot;
pub use jinn_theme_msg::theme_picker_scope;
pub use theme_picker_routes::open_from_scope as open_theme_picker_from_scope;

/// Activates the slice: scans both theme directories and mints the
/// theme-entries cell with the ordered results.
///
/// The built-in default is pinned first; a scanned theme named "default"
/// replaces the built-in's look while keeping its reserved slot; the rest
/// follow in case-insensitive name order. One unloadable file is noted on
/// stderr and skipped — a single bad file never drops the batch.
///
/// # Panics
///
/// Panics if the slot is already registered — double activation is a
/// wiring bug.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate(
    host: &mut SliceHost<'_, jinn_slices::RenderFacts>,
    themes_dir: &Path,
    system_themes_dir: &Path,
) {
    let entries = scan(themes_dir, system_themes_dir);
    let _cell = host
        .register_cell(theme_entries_slot(), entries)
        .expect("theme-entries slot is registered exactly once at wiring");
}

/// Registers the theme picker: its cell, its overlay, its keys, and its
/// filter hook.
///
/// Split from [`activate`] because the picker needs the same host but is not
/// part of theme discovery. Called from composition right after `activate`,
/// so the picker's rows are seeded from the entries cell `activate` just
/// minted.
///
/// # Panics
///
/// Panics if the picker slot is already registered — double activation is a
/// wiring bug.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate_picker(host: &mut SliceHost<'_, jinn_slices::RenderFacts>) {
    let cell = host
        .register_cell(
            jinn_theme_msg::theme_picker_slot(),
            jinn_theme_msg::ThemePickerState::default(),
        )
        .expect("theme picker slot is registered exactly once at wiring");

    let scope = theme_picker_scope();
    host.register_overlay(
        scope.clone(),
        std::sync::Arc::new(theme_picker_render::theme_picker_overlay_rect),
    );
    host.register_overlay_selectable(&scope);
    host.register_overlay_slot(scope.clone(), jinn_theme_msg::theme_picker_slot());
    host.register_overlay_view(
        scope,
        std::sync::Arc::new(theme_picker_render::render_theme_picker),
    );

    // The picker's keys, and the filter's input hook, are this slice's own.
    theme_picker_routes::attach_theme_picker_rows(host.key_routes(), &cell);
    theme_picker_routes::register_theme_picker_input_hook(host.key_routes(), &cell);
}

/// Scans both directories into the ordered selection.
///
/// The user directory shadows the system directory for same-name themes
/// (matching `load_theme`'s user-first lookup rule), so the system dir is
/// merged first and the user dir's entries overwrite them. Failures are
/// noted on stderr (host-side diagnostics) and never abort the batch.
#[must_use]
pub fn scan(themes_dir: &Path, system_themes_dir: &Path) -> ThemeEntries {
    let mut defs = std::collections::BTreeMap::new();
    for dir in [system_themes_dir, themes_dir] {
        match discover_themes(dir) {
            Ok(found) => merge_dir(&mut defs, found),
            Err(report) => note_scan_failure(dir, &report),
        }
    }

    let mut entries = vec![NamedTheme {
        name: "default".to_owned(),
        theme: jinn_theme::default_theme(),
    }];
    if let Some(default) = defs.remove("default")
        && let Some(pinned) = entries.first_mut()
    {
        pinned.theme = default;
    }
    entries.extend(
        defs.into_iter()
            .map(|(name, theme)| NamedTheme { name, theme }),
    );
    ThemeEntries { entries }
}

/// Merges one directory's discoveries into the accumulated set.
#[expect(
    clippy::print_stderr,
    reason = "host-side diagnostics: the theme loader runs before tracing exists"
)]
fn merge_dir(
    defs: &mut std::collections::BTreeMap<String, jinn_theme::Theme>,
    found: Vec<(String, std::path::PathBuf)>,
) {
    for (name, path) in found {
        match load_theme_from_file(&path) {
            Ok(theme) => {
                defs.insert(name, theme);
            }
            Err(report) => {
                eprintln!("theme-slice: skipping theme `{name}`: {report}");
            }
        }
    }
}

/// Notes a directory scan failure on stderr (host-side diagnostics).
#[expect(
    clippy::print_stderr,
    reason = "host-side diagnostics before tracing is available in the theme loader"
)]
fn note_scan_failure(dir: &Path, report: &error_stack::Report<jinn_theme::ThemeError>) {
    eprintln!("theme-slice: scan failed for {}: {report}", dir.display());
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::*;

    /// Writes one loadable theme TOML into `dir`.
    fn write_theme(dir: &Path, name: &str, accent: &str) {
        std::fs::create_dir_all(dir).expect("mkdir");
        std::fs::write(
            dir.join(format!("{name}.toml")),
            format!("focus_accent = \"{accent}\"\n"),
        )
        .expect("write theme");
    }

    #[rstest::rstest]
    fn scan_merges_both_dirs_with_default_first() {
        // Given a user dir with two themes and a system dir with one.
        let user = tempfile::tempdir().expect("tmpdir");
        let system = tempfile::tempdir().expect("tmpdir");
        write_theme(user.path(), "beta", "#ff0000");
        write_theme(user.path(), "alpha", "#00ff00");
        write_theme(system.path(), "gamma", "#0000ff");

        // When scanning.
        let entries = scan(user.path(), system.path());

        // Then default is pinned first and the rest follow in
        // case-insensitive name order.
        let names: Vec<_> = entries.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["default", "alpha", "beta", "gamma"]);
    }

    #[rstest::rstest]
    fn user_dir_shadows_system_dir_for_same_name() {
        // Given the same theme name in both dirs with different accents.
        let user = tempfile::tempdir().expect("tmpdir");
        let system = tempfile::tempdir().expect("tmpdir");
        write_theme(user.path(), "dup", "#ff0000");
        write_theme(system.path(), "dup", "#0000ff");

        // When scanning.
        let entries = scan(user.path(), system.path());

        // Then the user dir's colors win.
        let dup = entries.theme("dup").expect("dup present");
        assert_eq!(dup.focus_accent, "#ff0000".parse().expect("color"));
    }

    #[rstest::rstest]
    fn a_theme_named_default_replaces_the_builtin() {
        // Given a scanned theme named "default".
        let user = tempfile::tempdir().expect("tmpdir");
        write_theme(user.path(), "default", "#123456");

        // When scanning.
        let entries = scan(user.path(), system_dir_absent());

        // Then the pinned first entry carries the scanned look under the
        // reserved "default" name.
        assert_eq!(entries.entries[0].name, "default");
        assert_eq!(
            entries.entries[0].theme.focus_accent,
            "#123456".parse().expect("color")
        );
    }

    #[rstest::rstest]
    fn one_bad_file_never_drops_the_batch() {
        // Given one broken toml and one good toml.
        let user = tempfile::tempdir().expect("tmpdir");
        std::fs::create_dir_all(user.path()).expect("mkdir");
        std::fs::write(user.path().join("broken.toml"), "not [valid toml").expect("write");
        write_theme(user.path(), "good", "#00ff00");

        // When scanning.
        let entries = scan(user.path(), system_dir_absent());

        // Then only the good theme survives, alongside the pinned default.
        let names: Vec<_> = entries.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["default", "good"]);
    }

    #[rstest::rstest]
    fn missing_directories_yield_default_only() {
        // Given directories that do not exist.
        // When scanning both directories.
        let entries = scan(Path::new("/nonexistent-a"), Path::new("/nonexistent-b"));

        // Then the selection is the built-in default alone.
        assert_eq!(entries.entries.len(), 1);
        assert_eq!(entries.entries[0].name, "default");
    }

    /// A path guaranteed not to exist (keeps the missing-dir case explicit).
    fn system_dir_absent() -> &'static Path {
        Path::new("/nonexistent-system-themes")
    }
}
