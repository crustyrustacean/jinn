//! `workflow.md` frontmatter — the machine-readable identity of a workflow.
//!
//! A workflow's `workflow.md` begins with `+++`-delimited TOML frontmatter
//! naming the workflow and describing it. Parsing delegates to the one
//! shared TOML frontmatter parser ([`jinn_context::frontmatter::parse_toml_frontmatter`]);
//! this module is only the serde target over it plus the admission test.

use serde::Deserialize;
use wherror::Error;

/// The frontmatter a `workflow.md` may declare.
///
/// `name` is optional (the directory name is the fallback); `description`
/// is the admission requirement — a workflow without one is not
/// discoverable.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WorkflowFrontmatter {
    /// The workflow's display name. Absent → the directory name.
    #[serde(default)]
    pub name: Option<String>,
    /// The one-line description. Required, and must be non-blank, for the
    /// workflow to be admitted.
    #[serde(default)]
    pub description: Option<String>,
}

/// Why a `workflow.md` failed admission.
///
/// Admission is strict: every one of these means the workflow is skipped by
/// the scan entirely — never surfaced as an error, never partially shown.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error(debug)]
pub enum AdmissionError {
    /// The file has no `+++` block, or the block never closes.
    #[error(debug)]
    NoFrontmatter,
    /// The `+++` block is not valid TOML.
    #[error(debug)]
    MalformedFrontmatter,
    /// The `description` field is absent or whitespace-only.
    #[error(debug)]
    MissingDescription,
}

/// Parses a `workflow.md`'s frontmatter and applies the admission test.
///
/// Returns the frontmatter when the workflow is admissible; every failure
/// mode names exactly why it is not. A missing `name` is *not* a failure —
/// the directory name is the fallback at the scan site.
///
/// # Errors
///
/// Returns [`AdmissionError`] for a missing `+++` block, malformed TOML
/// inside it, or a missing/empty `description`.
pub fn parse_workflow_frontmatter(content: &str) -> Result<WorkflowFrontmatter, AdmissionError> {
    let (frontmatter, _body): (WorkflowFrontmatter, String) =
        jinn_context::frontmatter::parse_toml_frontmatter(content).map_err(
            |report| match report.current_context() {
                jinn_context::frontmatter::FrontmatterError::Parse => {
                    AdmissionError::MalformedFrontmatter
                }
                _ => AdmissionError::NoFrontmatter,
            },
        )?;

    match frontmatter.description.as_deref().map(str::trim) {
        None | Some("") => Err(AdmissionError::MissingDescription),
        Some(_) => Ok(frontmatter),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]

    use super::*;

    #[rstest::rstest]
    fn frontmatter_with_name_and_description_is_admitted() {
        // Given a workflow.md with valid frontmatter naming both fields.
        let content = "+++\nname = \"research-report\"\ndescription = \"Research a topic.\"\n+++\n\n## Flow\n";

        // When parsing the frontmatter.
        let fm = parse_workflow_frontmatter(content).expect("admitted");

        // Then both fields parse.
        assert_eq!(fm.name.as_deref(), Some("research-report"));
        assert_eq!(fm.description.as_deref(), Some("Research a topic."));
    }

    #[rstest::rstest]
    fn frontmatter_without_a_name_is_admitted() {
        // Given frontmatter carrying only a description.
        let content = "+++\ndescription = \"Has desc but no name\"\n+++\nbody";

        // When parsing.
        let fm = parse_workflow_frontmatter(content).expect("admitted");

        // Then the name is absent — the directory fallback applies at the
        // scan site — and the description is present.
        assert_eq!(fm.name, None);
        assert_eq!(fm.description.as_deref(), Some("Has desc but no name"));
    }

    #[rstest::rstest]
    fn a_file_without_frontmatter_is_rejected() {
        // Given a workflow.md that starts with markdown, no +++ block.
        let content = "# Name: legacy\n# Description: header-style\n\n## Flow\n";

        // When parsing.
        let result = parse_workflow_frontmatter(content);

        // Then admission fails with NoFrontmatter.
        assert_eq!(result.unwrap_err(), AdmissionError::NoFrontmatter);
    }

    #[rstest::rstest]
    fn malformed_frontmatter_toml_is_rejected() {
        // Given a +++ block whose body is not valid TOML.
        let content = "+++\nname = [unclosed\n+++\nbody";

        // When parsing.
        let result = parse_workflow_frontmatter(content);

        // Then admission fails with MalformedFrontmatter.
        assert_eq!(result.unwrap_err(), AdmissionError::MalformedFrontmatter);
    }

    #[rstest::rstest]
    fn an_unclosed_frontmatter_block_is_rejected() {
        // Given a +++ opener with no closer.
        let content = "+++\nname = \"dangling\"\nbody";

        // When parsing.
        let result = parse_workflow_frontmatter(content);

        // Then admission fails with NoFrontmatter.
        assert_eq!(result.unwrap_err(), AdmissionError::NoFrontmatter);
    }

    #[rstest::rstest]
    fn a_missing_description_is_rejected() {
        // Given frontmatter with only a name.
        let content = "+++\nname = \"no-desc\"\n+++\nbody";

        // When parsing.
        let result = parse_workflow_frontmatter(content);

        // Then admission fails with MissingDescription — never skip for a
        // missing name, only for a missing description.
        assert_eq!(result.unwrap_err(), AdmissionError::MissingDescription);
    }

    #[rstest::rstest]
    fn a_blank_description_is_rejected() {
        // Given frontmatter whose description is whitespace.
        let content = "+++\nname = \"blank\"\ndescription = \"   \"\n+++\nbody";

        // When parsing.
        let result = parse_workflow_frontmatter(content);

        // Then admission fails with MissingDescription.
        assert_eq!(result.unwrap_err(), AdmissionError::MissingDescription);
    }
}
