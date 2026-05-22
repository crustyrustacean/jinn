//! Fixture directory management for bench tasks.

#![allow(dead_code, reason = "used by runner in phase 4")]

use std::fs;
use std::io;
use std::path::Path;

/// Prepares a working directory for a bench task.
///
/// If `fixture_dir` is `Some`, copies the fixture contents into `target`.
/// If `None`, creates an empty `target` directory.
///
/// # Errors
///
/// Returns an error if the fixture source directory doesn't exist or if
/// any file operation fails.
pub fn prepare_fixture(fixture_dir: Option<&str>, target: &Path) -> io::Result<()> {
    fs::create_dir_all(target)?;
    if let Some(fixture) = fixture_dir {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(fixture);
        copy_dir_recursive(&source, target)?;
    }
    Ok(())
}

/// Recursively copies a directory tree from `src` into `dst`.
fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            fs::create_dir_all(&dst_path)?;
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
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
    fn prepare_fixture_copies_files() {
        // Given a source directory with a file.
        let root = tempfile::TempDir::new().expect("temp dir");
        let source = root.path().join("source");
        fs::create_dir_all(&source).expect("create source");
        fs::write(source.join("hello.txt"), "hello").expect("write");

        let target = root.path().join("target");

        // When copying.
        copy_dir_recursive(&source, &target).expect("copy");

        // Then the file is present in target.
        assert!(target.join("hello.txt").exists());
        assert_eq!(
            fs::read_to_string(target.join("hello.txt")).expect("read"),
            "hello"
        );
    }

    #[test]
    fn prepare_fixture_copies_nested_dirs() {
        // Given a source directory with nested structure.
        let root = tempfile::TempDir::new().expect("temp dir");
        let source = root.path().join("source");
        let nested = source.join("a/b");
        fs::create_dir_all(&nested).expect("create nested");
        fs::write(nested.join("deep.txt"), "deep").expect("write");

        let target = root.path().join("target");

        // When copying.
        copy_dir_recursive(&source, &target).expect("copy");

        // Then the nested file is present.
        assert!(target.join("a/b/deep.txt").exists());
    }
}
