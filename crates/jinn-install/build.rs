//! Build script: makes `include_dir!("$CARGO_MANIFEST_DIR/../../res")`
//! re-expand whenever the embedded resource tree changes *in shape*.
//!
//! `include_dir!` expands each file to a real `include_bytes!`, so editing a
//! file already in the tree re-triggers a rebuild on its own — but *adding*,
//! *moving*, or *deleting* one does not. Without this script a file dropped
//! into `res/` would never ship: the macro's expansion is cached and the
//! binary keeps the old tree. Cargo has no "watch this directory recursively"
//! directive, so every path currently under `res/` is declared individually;
//! a new file under a watched directory is a path cargo has not seen, so the
//! enclosing directories are declared too.
//!
//! (`include_dir`'s `nightly` feature solves this with `track_path`, but the
//! repo builds on stable and stays there.)
//!
//! Declared paths are relative to this crate's manifest directory, never to the
//! cwd. A rerun-if-changed path that does not exist makes cargo treat the
//! build script as always-dirty — recompiling on every build, silently and
//! permanently — so the build aborts instead.

#![allow(warnings, reason = "want to fail fast")]

use std::path::{Path, PathBuf};

/// The resource tree embedded by `jinn-install`.
const RES: &str = "../../res";

fn main() {
    let res = manifest_dir().join(RES);

    assert!(
        res.is_dir(),
        "build script rerun path is not a directory: {} (resolved from CARGO_MANIFEST_DIR)",
        res.display()
    );

    watch_tree(&res);
}

/// Emits `cargo:rerun-if-changed` for every path under `root`, files and
/// directories alike.
fn watch_tree(root: &Path) {
    let entries = std::fs::read_dir(root)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", root.display()));

    let mut paths: Vec<PathBuf> = entries
        .map(|entry| {
            entry
                .unwrap_or_else(|e| panic!("failed to read entry in {}: {e}", root.display()))
                .path()
        })
        .collect();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            watch_tree(&path);
        }
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
}
