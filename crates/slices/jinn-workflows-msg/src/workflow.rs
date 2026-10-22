//! Portable workflow values shared by the workflow messages and the
//! workflows slice implementation.

use marked_path::{Absolute, MarkedPathBuf};
use serde::{Deserialize, Serialize};

/// A discovered workflow.
///
/// Parsed from a `workflow.md` file with valid `+++` TOML frontmatter in a
/// direct subdirectory of the workflows directory. A directory whose
/// `workflow.md` lacks frontmatter, has malformed frontmatter, or has a
/// missing/empty `description` is not admitted — it never becomes a
/// [`Workflow`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Workflow {
    /// The workflow's display name: the frontmatter `name` when present,
    /// otherwise the directory name.
    pub name: String,
    /// The frontmatter description. Admission requires it to be present and
    /// non-blank, so this is never empty.
    pub description: String,
    /// Absolute path to the workflow's directory (the one holding
    /// `workflow.md` and `steps/`).
    pub dir: MarkedPathBuf<Absolute>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]

    use super::*;

    #[rstest::rstest]
    fn workflow_roundtrips_through_serde() {
        // Given a workflow with every field set. The directory is built from
        // a temp root, so it is an absolute path on every platform.
        let root = jinn_testutil::tempdir();
        let workflow = Workflow {
            name: "research-report".to_owned(),
            description: "Research a topic and deliver a cited brief.".to_owned(),
            dir: root
                .path
                .join_all(["workflows", "research-report"])
                .expect("a"),
        };

        // When serializing then deserializing.
        let json = serde_json::to_string(&workflow).expect("serialize");
        let back: Workflow = serde_json::from_str(&json).expect("deserialize");

        // Then the fields survive the roundtrip.
        assert_eq!(back, workflow);
    }
}
