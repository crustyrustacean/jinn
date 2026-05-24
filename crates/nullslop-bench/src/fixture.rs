//! Fixture directory management for bench tasks.
//!
//! Embeds fixture directories into the binary at compile time using
//! `include_dir!`, then extracts them to disk at runtime during setup.

use std::fs;
use std::io;
use std::path::Path;

use include_dir::Dir;

/// Prepares a working directory for a bench task.
///
/// If `fixture_dir` is `Some`, extracts the embedded fixture contents into
/// `target`. If `None`, creates an empty `target` directory.
///
/// # Errors
///
/// Returns an error if any file operation fails.
pub fn prepare_fixture(
    fixture_dir: Option<&'static Dir<'static>>,
    target: &Path,
) -> io::Result<()> {
    fs::create_dir_all(target)?;
    if let Some(dir) = fixture_dir {
        extract_dir(dir, target)?;
    }
    Ok(())
}

/// Recursively extracts an embedded directory to disk.
fn extract_dir(dir: &Dir<'_>, target: &Path) -> io::Result<()> {
    for entry in dir.entries() {
        match entry {
            include_dir::DirEntry::Dir(sub_dir) => {
                // Sub-directory files have paths relative to the root Dir,
                // so extract into the original target, not a sub-path.
                extract_dir(sub_dir, target)?;
            }
            include_dir::DirEntry::File(file) => {
                let file_path = target.join(file.path());
                // Ensure parent directory exists.
                if let Some(parent) = file_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&file_path, file.contents())?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]
    #![allow(clippy::indexing_slicing, reason = "test code")]

    use super::*;

    #[test]
    fn prepare_fixture_with_none_creates_empty_dir() {
        // Given a temp root.
        let root = tempfile::TempDir::new().expect("temp dir");
        let target = root.path().join("work");

        // When preparing with no fixture.
        prepare_fixture(None, &target).expect("prepare");

        // Then the target exists and is empty.
        assert!(target.is_dir());
        assert!(fs::read_dir(&target).expect("read dir").count() == 0);
    }

    #[test]
    fn prepare_fixture_extracts_embedded_files() {
        // Given an embedded fixture directory with files.
        static FIXTURES: Dir<'_> = include_dir::include_dir!(
            "$CARGO_MANIFEST_DIR/src/tasks/fix_code/fix_syntax_broken_rust/fixtures"
        );

        let root = tempfile::TempDir::new().expect("temp dir");
        let target = root.path().join("work");

        // When extracting.
        prepare_fixture(Some(&FIXTURES), &target).expect("prepare");

        // Then the fixture files are present.
        assert!(target.join("Cargo.toml").exists());
        assert!(target.join("src/main.rs").exists());
    }

    #[test]
    fn prepare_fixture_extracts_nested_dirs() {
        // Given an embedded fixture with nested structure.
        static FIXTURES: Dir<'_> = include_dir::include_dir!(
            "$CARGO_MANIFEST_DIR/src/tasks/edit/edit_multi_file_refactor/fixtures"
        );

        let root = tempfile::TempDir::new().expect("temp dir");
        let target = root.path().join("work");

        // When extracting.
        prepare_fixture(Some(&FIXTURES), &target).expect("prepare");

        // Then nested files are present.
        assert!(target.join("Cargo.toml").exists());
        assert!(target.join("src/lib.rs").exists());
        assert!(target.join("src/main.rs").exists());
    }
}
