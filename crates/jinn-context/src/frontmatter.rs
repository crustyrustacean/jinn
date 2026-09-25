//! TOML frontmatter parser for `+++`-delimited content blocks.
//!
//! Extracts frontmatter and body from content like:
//!
//! ```markdown
//! +++
//! name = "example"
//! description = "..."
//! +++
//! Body text here...
//! ```

use error_stack::ResultExt as _;
use serde::de::DeserializeOwned;
use wherror::Error;

/// Error during frontmatter parsing.
#[derive(Debug, Error)]
#[error(debug)]
pub enum FrontmatterError {
    /// Content does not start with `+++` delimiter.
    MissingOpenDelimiter,
    /// No closing `+++` delimiter found.
    MissingCloseDelimiter,
    /// TOML parsing failed.
    Parse,
}

/// Parses `+++`-delimited TOML frontmatter from a content string.
///
/// Returns `(frontmatter, body)` where frontmatter is the deserialized TOML
/// and body is the text after the closing `+++`.
///
/// # Errors
///
/// Returns [`FrontmatterError`] if the content is malformed.
pub fn parse_toml_frontmatter<T>(
    content: &str,
) -> Result<(T, String), error_stack::Report<FrontmatterError>>
where
    T: DeserializeOwned,
{
    let trimmed = content.trim_start();

    let Some(after_open) = trimmed.strip_prefix("+++") else {
        return Err(
            error_stack::Report::new(FrontmatterError::MissingOpenDelimiter)
                .attach("content must start with +++ frontmatter delimiter"),
        );
    };

    let Some((frontmatter_str, body_rest)) = after_open.split_once("\n+++") else {
        return Err(
            error_stack::Report::new(FrontmatterError::MissingCloseDelimiter)
                .attach("missing closing +++ frontmatter delimiter"),
        );
    };

    let frontmatter_str = frontmatter_str.trim();
    let body = body_rest.trim_start_matches('\n').trim_end().to_owned();

    let frontmatter: T = toml::from_str(frontmatter_str)
        .change_context(FrontmatterError::Parse)
        .attach("failed to parse frontmatter TOML")?;

    Ok((frontmatter, body))
}
