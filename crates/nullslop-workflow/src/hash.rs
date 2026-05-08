//! Content hashing for file outputs.
//!
//! Provides SHA-256 content hashing used by the workflow system for invalidation
//! tracking. When a step completes, its file outputs are hashed. On jump-back,
//! hashes are compared to determine if downstream steps are still valid.

use std::path::Path;

use sha2::{Digest as _, Sha256};

/// Compute a SHA-256 content hash for a file.
///
/// Returns the hex-encoded hash string, or `None` if the file does not exist
/// or cannot be read.
pub fn file_content_hash(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let result = hasher.finalize();
    Some(format!("{result:x}"))
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    #[rstest::rstest]    fn same_content_produces_same_hash() {
        // Given a file with known content.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"hello world").unwrap();
        }

        // When hashing the file twice.
        let hash1 = file_content_hash(&path).unwrap();
        let hash2 = file_content_hash(&path).unwrap();

        // Then both hashes are identical.
        assert_eq!(hash1, hash2);
    }

    #[rstest::rstest]    fn different_content_produces_different_hash() {
        // Given two files with different content.
        let dir = tempfile::tempdir().unwrap();
        let path1 = dir.path().join("a.txt");
        let path2 = dir.path().join("b.txt");
        {
            let mut f1 = std::fs::File::create(&path1).unwrap();
            f1.write_all(b"content a").unwrap();
            let mut f2 = std::fs::File::create(&path2).unwrap();
            f2.write_all(b"content b").unwrap();
        }

        // When hashing both files.
        let hash1 = file_content_hash(&path1).unwrap();
        let hash2 = file_content_hash(&path2).unwrap();

        // Then the hashes differ.
        assert_ne!(hash1, hash2);
    }

    #[rstest::rstest]    fn nonexistent_file_returns_none() {
        // Given a path that does not exist.
        // When computing the hash.
        let hash = file_content_hash(Path::new("/nonexistent/file/that/does/not/exist.txt"));

        // Then the result is None.
        assert!(hash.is_none());
    }
}
