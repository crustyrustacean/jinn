//! Plugin directory scanner.
//!
//! Scans a directory for subdirectories containing `init.lua` files,
//! skipping hidden directories and files.

use std::path::{Path, PathBuf};

/// Scans `plugins_dir` for plugin directories.
///
/// A plugin directory is a non-hidden subdirectory that contains an `init.lua`
/// file. Returns paths sorted alphabetically by directory name.
///
/// # Errors
///
/// Returns an error if `plugins_dir` does not exist or is not a directory.
pub fn scan(plugins_dir: &Path) -> Result<Vec<PathBuf>, ScanError> {
    if !plugins_dir.is_dir() {
        return Err(ScanError::NotADirectory {
            path: plugins_dir.to_path_buf(),
        });
    }

    let entries = plugins_dir
        .read_dir()
        .map_err(|e| ScanError::Io {
            path: plugins_dir.to_path_buf(),
            source: e,
        })?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // Skip hidden directories.
            if name_str.starts_with('.') {
                return None;
            }

            let path = entry.path();
            if path.is_dir() && path.join("init.lua").is_file() {
                return Some(path);
            }

            None
        })
        .collect::<Vec<_>>();

    let mut sorted = entries;
    sorted.sort();

    Ok(sorted)
}

/// Errors from scanning a plugin directory.
#[derive(Debug)]
pub enum ScanError {
    /// The provided path is not a directory.
    NotADirectory {
        /// The invalid path.
        path: PathBuf,
    },
    /// An I/O error occurred.
    Io {
        /// The path being read.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotADirectory { path } => {
                write!(f, "not a directory: {}", path.display())
            }
            Self::Io { path, source } => {
                write!(f, "I/O error reading {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for ScanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::NotADirectory { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        reason = "test code, panics are acceptable"
    )]
    use std::fs;

    use super::*;

    #[rstest::rstest]
    fn scan_finds_init_lua_in_plugin_dirs() {
        // Given a plugins directory with 3 valid plugin subdirectories.
        let dir = tempfile::tempdir().expect("create temp dir");
        for name in ["alpha", "beta", "gamma"] {
            let plugin_dir = dir.path().join(name);
            fs::create_dir_all(&plugin_dir).expect("create plugin dir");
            fs::write(plugin_dir.join("init.lua"), "-- plugin").expect("write init.lua");
        }

        // When scanning.
        let result = scan(dir.path()).expect("scan succeeds");

        // Then it finds 3 plugins sorted alphabetically.
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].file_name().expect("name"), "alpha");
        assert_eq!(result[1].file_name().expect("name"), "beta");
        assert_eq!(result[2].file_name().expect("name"), "gamma");
    }

    #[rstest::rstest]
    fn scan_ignores_dirs_without_init_lua() {
        // Given a plugins directory with one valid and one invalid subdirectory.
        let dir = tempfile::tempdir().expect("create temp dir");

        let valid = dir.path().join("valid");
        fs::create_dir_all(&valid).expect("create dir");
        fs::write(valid.join("init.lua"), "-- plugin").expect("write init.lua");

        let invalid = dir.path().join("invalid");
        fs::create_dir_all(&invalid).expect("create dir");
        fs::write(invalid.join("readme.txt"), "not a plugin").expect("write readme");

        // When scanning.
        let result = scan(dir.path()).expect("scan succeeds");

        // Then only the valid plugin is found.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].file_name().expect("name"), "valid");
    }

    #[rstest::rstest]
    fn scan_ignores_hidden_dirs() {
        // Given a plugins directory with a hidden subdirectory containing init.lua.
        let dir = tempfile::tempdir().expect("create temp dir");

        let hidden = dir.path().join(".hidden");
        fs::create_dir_all(&hidden).expect("create dir");
        fs::write(hidden.join("init.lua"), "-- hidden plugin").expect("write init.lua");

        // When scanning.
        let result = scan(dir.path()).expect("scan succeeds");

        // Then the hidden directory is skipped.
        assert!(result.is_empty(), "hidden directories should be skipped");
    }
}
