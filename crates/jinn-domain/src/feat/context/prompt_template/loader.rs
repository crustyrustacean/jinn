//! Prompt template file parser - reads markdown files with TOML frontmatter.
//!
//! Frontmatter is delimited by `+++` on its own line:
//!
//! ```markdown
//! +++
//! name = "code-review"
//! description = "Perform a thorough code review"
//! +++
//! Template body here...
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use crate::protocol::PromptTemplate;
use error_stack::{Report, ResultExt as _};
use serde::Deserialize;

/// Errors that can occur during template file parsing.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub enum PromptTemplateParseError {
    /// Filesystem I/O failure.
    Io,
    /// TOML frontmatter is missing or malformed.
    Frontmatter,
    /// TOML parsing error.
    Parse,
}

/// Frontmatter schema - the metadata extracted from between `+++` delimiters.
#[derive(Debug, Deserialize)]
struct Frontmatter {
    /// Unique template name.
    name: String,
    /// Short description.
    #[serde(default)]
    description: String,
}

/// Parses a single markdown file into a [`PromptTemplate`].
///
/// Expects the file to start with `+++`, contain TOML frontmatter, end with `+++`,
/// and have the template body after the closing delimiter.
///
/// # Errors
///
/// Returns an error if the file cannot be read, the frontmatter is missing or
/// malformed, or the TOML cannot be parsed.
pub fn parse_template_file(
    path: &Path,
) -> Result<PromptTemplate, Report<PromptTemplateParseError>> {
    let content = std::fs::read_to_string(path)
        .change_context(PromptTemplateParseError::Io)
        .attach(format!("failed to read {}", path.display()))?;

    parse_template_content(&content)
}

/// Renders a [`PromptTemplate`] into the markdown file format with TOML frontmatter.
///
/// Produces a string like:
///
/// ```markdown
/// +++
/// name = "example"
/// description = "..."
/// +++
/// Template body...
/// ```
///
/// Builds frontmatter from the struct fields using a `BTreeMap` so that adding
/// new fields to [`PromptTemplate`] automatically appears in the output.
///
/// # Panics
///
/// Panics if TOML serialization fails, which should not happen with simple string values.
#[must_use]
#[expect(
    clippy::expect_used,
    reason = "BTreeMap<String, String> serialization is infallible"
)]
pub fn render_template_file(template: &PromptTemplate) -> String {
    let mut frontmatter = BTreeMap::new();
    frontmatter.insert("name", template.name.clone());
    frontmatter.insert("description", template.description.clone());

    let toml_str =
        toml::to_string(&frontmatter).expect("serializing frontmatter BTreeMap cannot fail");

    format!("+++\n{toml_str}+++\n{}", template.body)
}

/// Parses template content (extracted for testability without touching disk).
pub(crate) fn parse_template_content(
    content: &str,
) -> Result<PromptTemplate, Report<PromptTemplateParseError>> {
    let (frontmatter, body) =
        crate::common::frontmatter::parse_toml_frontmatter::<Frontmatter>(content)
            .change_context(PromptTemplateParseError::Frontmatter)
            .attach("failed to parse template file")?;

    Ok(PromptTemplate {
        name: frontmatter.name,
        description: frontmatter.description,
        body,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;

    #[rstest::rstest]
    fn parses_valid_template() {
        // Given a well-formed template file.
        let content = "+++\nname = \"hello\"\ndescription = \"Say hello\"\n+++\nHello, world!";

        // When parsing.
        let template = parse_template_content(content).expect("parse");

        // Then the fields are correct.
        assert_eq!(template.name, "hello");
        assert_eq!(template.description, "Say hello");
        assert_eq!(template.body, "Hello, world!");
    }

    #[rstest::rstest]
    fn parses_template_without_description() {
        // Given a template with no description field.
        let content = "+++\nname = \"minimal\"\n+++\nJust the body.";

        // When parsing.
        let template = parse_template_content(content).expect("parse");

        // Then description defaults to empty and body is correct.
        assert_eq!(template.name, "minimal");
        assert_eq!(template.description, "");
        assert_eq!(template.body, "Just the body.");
    }

    #[rstest::rstest]
    fn parses_template_with_multiline_body() {
        // Given a template with a multi-line body.
        let content = "+++\nname = \"review\"\ndescription = \"Code review\"\n+++\nLine one.\nLine two.\nLine three.";

        // When parsing.
        let template = parse_template_content(content).expect("parse");

        // Then the body preserves all lines.
        assert_eq!(template.body, "Line one.\nLine two.\nLine three.");
    }

    #[rstest::rstest]
    fn fails_without_opening_delimiter() {
        // Given content without frontmatter.
        let content = "name = \"hello\"\n+++\nBody";

        // When parsing.
        let result = parse_template_content(content);

        // Then it fails with a frontmatter error.
        assert!(result.is_err());
    }

    #[rstest::rstest]
    fn fails_without_closing_delimiter() {
        // Given content with only an opening delimiter.
        let content = "+++\nname = \"hello\"";

        // When parsing.
        let result = parse_template_content(content);

        // Then it fails with a frontmatter error.
        assert!(result.is_err());
    }

    #[rstest::rstest]
    fn fails_with_invalid_toml() {
        // Given content with invalid TOML in the frontmatter.
        let content = "+++\nname = invalid\n+++\nBody";

        // When parsing.
        let result = parse_template_content(content);

        // Then it fails with a parse error.
        assert!(result.is_err());
    }

    #[rstest::rstest]
    fn handles_leading_whitespace() {
        // Given content with leading whitespace before +++.
        let content = "\n\n+++\nname = \"hello\"\n+++\nBody here.";

        // When parsing.
        let template = parse_template_content(content).expect("parse");

        // Then it parses correctly.
        assert_eq!(template.name, "hello");
        assert_eq!(template.body, "Body here.");
    }

    #[rstest::rstest]
    fn handles_empty_body() {
        // Given a template with no body after the closing delimiter.
        let content = "+++\nname = \"empty\"\n+++\n";

        // When parsing.
        let template = parse_template_content(content).expect("parse");

        // Then the body is empty.
        assert_eq!(template.name, "empty");
        assert_eq!(template.body, "");
    }

    #[rstest::rstest]
    fn render_then_parse_round_trips() {
        // Given a template with all fields populated.
        let original = PromptTemplate {
            name: "example".to_owned(),
            description: "An example template".to_owned(),
            body: "You are a helpful assistant.".to_owned(),
        };

        // When rendering to file format and parsing back.
        let rendered = render_template_file(&original);
        let parsed = parse_template_content(&rendered).expect("round-trip parse");

        // Then the parsed template matches the original.
        assert_eq!(parsed.name, original.name);
        assert_eq!(parsed.description, original.description);
        assert_eq!(parsed.body, original.body);
    }

    #[rstest::rstest]
    fn render_includes_all_frontmatter_fields() {
        // Given a template.
        let template = PromptTemplate {
            name: "test".to_owned(),
            description: "desc".to_owned(),
            body: "body".to_owned(),
        };

        // When rendering.
        let rendered = render_template_file(&template);

        // Then the output contains both frontmatter fields and delimiters.
        assert!(rendered.starts_with("+++\n"));
        assert!(rendered.contains("name = \"test\""));
        assert!(rendered.contains("description = \"desc\""));
        assert!(rendered.contains("\n+++\n"));
        assert!(rendered.ends_with("body"));
    }
}
