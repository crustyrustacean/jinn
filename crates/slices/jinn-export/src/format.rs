//! The export format boundary.
//!
//! An [`ExportFormat`] turns an [`ExportDocument`] into a string. The
//! destination path's extension picks the format: `.html`/`.htm` render
//! HTML, `.md`/`.markdown` render markdown, and anything else is refused
//! with a message naming the supported extensions.
//!
//! This module is the only place that knows the extension set. Adding a
//! format is one trait impl plus one arm in [`format_for_path`]; no call
//! site outside a renderer's own file names a concrete format type.

use std::path::Path;

use wherror::Error;

use crate::document::ExportDocument;
use crate::html::HtmlExport;
use crate::markdown::MarkdownExport;

/// The extension used when the user gives no path at all.
pub const DEFAULT_EXTENSION: &str = "html";

/// Every extension the export slice can write, in the order they are shown
/// to the user when refusing an unsupported one.
const SUPPORTED_EXTENSIONS: &[&str] = &["html", "htm", "md", "markdown"];

/// A rendering of a session document into one file's contents.
///
/// Object-safe so the actor holds a boxed format chosen at runtime. A format
/// knows nothing about paths, the filesystem, or the session: it is a pure
/// function from document to bytes-as-text.
pub trait ExportFormat {
    /// The canonical file extension for this format, without the dot.
    fn extension(&self) -> &'static str;

    /// Renders the document into the full text of one file.
    fn render(&self, doc: &ExportDocument) -> String;
}

/// The path's extension names a format this slice cannot write.
///
/// Carries a user-facing message naming the extension and the supported set,
/// which the actor pushes into an `Error` chat entry. No file is written
/// when this fires.
#[derive(Debug, Error)]
#[error(debug)]
pub struct UnsupportedExtensionError {
    /// The extension that was refused, verbatim including the dot, or
    /// `(none)` when the path had none.
    pub found: String,
    /// A ready-to-render sentence naming the supported extensions.
    pub reason: String,
}

/// Picks the format for a resolved destination path.
///
/// Returns `None` when the extension is not one this slice can write; the
/// caller turns that into an [`UnsupportedExtensionError`]. This is the only
/// function that maps an extension to a format.
#[must_use]
pub fn format_for_path(path: &Path) -> Option<Box<dyn ExportFormat>> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => Some(Box::new(HtmlExport)),
        "md" | "markdown" => Some(Box::new(MarkdownExport)),
        _ => None,
    }
}

/// Builds the error for a path whose extension cannot be written.
#[must_use]
pub fn unsupported_extension_error(path: &Path) -> UnsupportedExtensionError {
    let found = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .map_or_else(|| "(none)".to_owned(), |e| format!(".{e}"));
    let supported = SUPPORTED_EXTENSIONS
        .iter()
        .map(|e| format!(".{e}"))
        .collect::<Vec<_>>()
        .join(", ");
    UnsupportedExtensionError {
        found: found.clone(),
        reason: format!("no exporter for {found} (supported: {supported})"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;

    #[rstest::rstest]
    #[case("html", "html")]
    #[case("htm", "html")]
    #[case("HTML", "html")]
    #[case("md", "md")]
    #[case("markdown", "md")]
    fn supported_extension_resolves_to_a_format(#[case] ext: &str, #[case] expected: &str) {
        // Given a path with a supported extension.
        let path = Path::new("out").with_extension(ext);

        // When resolving its format.
        let format = format_for_path(&path);

        // Then a format is found whose canonical extension matches.
        let format = format.expect("supported extension resolves");
        assert_eq!(format.extension(), expected);
    }

    #[rstest::rstest]
    #[case("pdf")]
    #[case("txt")]
    #[case("rs")]
    fn unsupported_extension_resolves_to_none(#[case] ext: &str) {
        // Given a path with an extension this slice cannot write.
        let path = Path::new("out").with_extension(ext);

        // When resolving its format.
        let format = format_for_path(&path);

        // Then nothing is found.
        assert!(format.is_none());
    }

    #[rstest::rstest]
    fn extensionless_path_resolves_to_none() {
        // Given a path with no extension at all.
        let path = Path::new("chat-export");

        // When resolving its format.
        let format = format_for_path(path);

        // Then nothing is found, so the export is refused.
        assert!(format.is_none());
    }

    #[rstest::rstest]
    fn unsupported_error_names_the_extension_and_the_alternatives() {
        // Given a path the slice cannot write.
        let path = Path::new("out.pdf");

        // When building the refusal error.
        let err = unsupported_extension_error(path);

        // Then the message says there is no exporter, and lists the ones there are.
        assert_eq!(err.found, ".pdf");
        assert!(err.reason.starts_with("no exporter for .pdf"));
        assert!(err.reason.contains(".html"));
        assert!(err.reason.contains(".md"));
    }

    #[rstest::rstest]
    fn extensionless_error_reports_none_found() {
        // Given a path with no extension.
        let path = Path::new("chat-export");

        // When building the refusal error.
        let err = unsupported_extension_error(path);

        // Then the message says no extension was found.
        assert_eq!(err.found, "(none)");
        assert!(err.reason.contains("(none)"));
    }
}
