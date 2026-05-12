//! Seed persona creation — ensures the personas directory has a default persona.

use std::path::Path;

use error_stack::{Report, ResultExt as _};
use wherror::Error;

use super::{SEED_FILENAME, personas_dir, seed_content};

/// Errors during seed persona creation.
#[derive(Debug, Error)]
#[error(debug)]
pub enum EnsurePersonaError {
    /// Filesystem I/O failure.
    Io,
}

/// Ensures the personas directory exists and contains the seed persona.
///
/// Creates `~/.config/nullslop/personas/` if missing, then writes the
/// seed "coding-assistant" persona if it doesn't already exist.
///
/// # Errors
///
/// Returns an error if the directory or file cannot be created due to I/O failure.
pub fn ensure_personas_dir_with_seed() -> Result<(), Report<EnsurePersonaError>> {
    let dir = personas_dir();
    ensure_personas_dir_with_seed_to(&dir)
}

/// Testable version that writes to an explicit directory.
pub(crate) fn ensure_personas_dir_with_seed_to(
    dir: &Path,
) -> Result<(), Report<EnsurePersonaError>> {
    std::fs::create_dir_all(dir)
        .change_context(EnsurePersonaError::Io)
        .attach("failed to create personas directory")?;

    let seed_path = dir.join(SEED_FILENAME);

    if seed_path.exists() {
        return Ok(());
    }

    std::fs::write(&seed_path, seed_content())
        .change_context(EnsurePersonaError::Io)
        .attach(format!("failed to write {}", seed_path.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn ensure_creates_directory() {
        // Given a nonexistent directory.
        let dir = tempfile::TempDir::new().expect("temp dir");
        let personas = dir.path().join("personas");
        assert!(!personas.exists());

        // When ensuring the personas dir with seed.
        ensure_personas_dir_with_seed_to(&personas).expect("ensure");

        // Then the directory exists.
        assert!(personas.exists());
    }

    #[rstest::rstest]
    fn ensure_creates_seed_file() {
        // Given a nonexistent directory.
        let dir = tempfile::TempDir::new().expect("temp dir");
        let personas = dir.path().join("personas");

        // When ensuring the personas dir with seed.
        ensure_personas_dir_with_seed_to(&personas).expect("ensure");

        // Then the seed file exists.
        let seed = personas.join(SEED_FILENAME);
        assert!(seed.exists());
    }

    #[rstest::rstest]
    fn seed_file_is_valid_persona() {
        // Given a nonexistent directory.
        let dir = tempfile::TempDir::new().expect("temp dir");
        let personas = dir.path().join("personas");

        // When ensuring the personas dir with seed.
        ensure_personas_dir_with_seed_to(&personas).expect("ensure");

        // Then the file parses back to a valid persona.
        let seed = personas.join(SEED_FILENAME);
        let content = std::fs::read_to_string(&seed).expect("read");
        let persona = super::super::loader::parse_persona_content(&content, &seed).expect("parse");
        assert_eq!(persona.name, "coding-assistant");
    }

    #[rstest::rstest]
    fn ensure_does_not_overwrite_existing_file() {
        // Given a directory that already has a seed file.
        let dir = tempfile::TempDir::new().expect("temp dir");
        let personas = dir.path().join("personas");
        std::fs::create_dir_all(&personas).expect("create dir");
        std::fs::write(personas.join(SEED_FILENAME), "custom content").expect("write");

        // When ensuring the personas dir with seed.
        ensure_personas_dir_with_seed_to(&personas).expect("ensure");

        // Then the existing file is not overwritten.
        let content = std::fs::read_to_string(personas.join(SEED_FILENAME)).expect("read");
        assert_eq!(content, "custom content");
    }

    #[rstest::rstest]
    fn ensure_is_idempotent() {
        // Given a personas directory.
        let dir = tempfile::TempDir::new().expect("temp dir");
        let personas = dir.path().join("personas");

        // When ensuring twice.
        ensure_personas_dir_with_seed_to(&personas).expect("first ensure");
        ensure_personas_dir_with_seed_to(&personas).expect("second ensure");

        // Then exactly one seed file exists with valid content.
        let seed = personas.join(SEED_FILENAME);
        let content = std::fs::read_to_string(&seed).expect("read");
        let persona = super::super::loader::parse_persona_content(&content, &seed).expect("parse");
        assert_eq!(persona.name, "coding-assistant");
    }
}
