//! The file an attendant was loaded from.

use serde::{Deserialize, Serialize};

/// The workflow-local TOML file an attendant was loaded from.
///
/// A path, not a name — the attendant's name is its title. This is the
/// provenance stamp `load_attendant` writes: the save path refuses to
/// write an attendant that carries one, naming this file, and the tool
/// result names it so the caller knows where the configuration came from.
///
/// Held as a `String` rather than a `PathBuf` because it is display text
/// first (it appears in refusals and tool results) and a filesystem path
/// second; it also rides persistence blobs, where a stable string form is
/// the whole contract.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AttendantSourcePath(String);

impl From<&str> for AttendantSourcePath {
    fn from(path: &str) -> Self {
        Self(path.to_owned())
    }
}

impl From<String> for AttendantSourcePath {
    fn from(path: String) -> Self {
        Self(path)
    }
}

impl AttendantSourcePath {
    /// A source path for the file at `path`.
    #[must_use]
    pub fn new<P>(path: P) -> Self
    where
        P: Into<String>,
    {
        Self(path.into())
    }

    /// The path as displayed in refusals and tool results.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AttendantSourcePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use super::*;

    #[rstest::rstest]
    fn a_source_path_displays_as_the_path_it_was_built_from() {
        // Given a source path built from a workflow-local file.
        let source = AttendantSourcePath::new("res/workflows/sample/attendants/qa.toml");

        // When displaying it.
        let displayed = source.to_string();

        // Then the display form is the path verbatim.
        assert_eq!(displayed, "res/workflows/sample/attendants/qa.toml");
    }

    #[rstest::rstest]
    fn a_source_path_serializes_and_deserializes_verbatim() {
        // Given a source path.
        let source = AttendantSourcePath::new("attendants/qa.toml");

        // When serializing and deserializing.
        let json = serde_json::to_string(&source).expect("serialize");
        let round: AttendantSourcePath = serde_json::from_str(&json).expect("deserialize");

        // Then the roundtrip is verbatim, so persistence is stable.
        assert_eq!(round.as_str(), "attendants/qa.toml");
    }
}
