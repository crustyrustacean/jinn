#![expect(clippy::expect_used, reason = "infallible static regex initialization")]
//! `@path` attachment scanning — detects `@path` tokens in user text,
//! reads the referenced image files, and rewrites the tokens to `file://` URIs.
//!
//! This runs as a second expansion pass after `#token` expansion (see
//! [`super::expand_tokens`]). Each `@path` reference is resolved against the
//! session CWD and replaced in place by `(file:///resolved/absolute/path)` so
//! that terminals with OSC-8 link support auto-link the original file.
//!
//! Accepted forms (each at a word boundary — start of buffer or preceded by
//! space/newline):
//! - `@/abs/path` — absolute path, used as-is.
//! - `@~/rel/path` — tilde-expanded against the home directory.
//! - `@rel/path` — resolved against the session CWD.
//!
//! `foo@path` (no boundary) is never matched. Non-image files (or missing
//! files) are left as their literal `@path` token.
//!
//! **Spaces in filenames are not supported.** The path runs to the next
//! whitespace, so `@my file.png` attaches only `@my` (which fails) and leaves
//! `file.png` as plain text.
//!
//! Confirms file existence (a missing or unreadable file is left untouched as
//! a literal `@path`). It does **not** validate that the file is a real image —
//! that is out of scope and handled at the provider edge.

use std::path::{Path, PathBuf};

use std::sync::LazyLock;

use regex::Regex;

use jinn_provider::Attachment;

/// Resolver context for turning raw `@path` tokens into absolute paths.
///
/// Relative paths resolve against `cwd`; `~`-prefixed paths resolve against
/// `home`. Absolute paths (`/...`) are used as-is.
#[derive(Debug, Clone, Copy)]
pub struct PathResolveContext<'a> {
    /// The session's working directory — base for relative `@rel/path` tokens.
    pub cwd: &'a Path,
    /// The user's home directory — base for `@~/path` (tilde) tokens.
    pub home: &'a Path,
}

impl<'a> PathResolveContext<'a> {
    /// Builds a resolver from a cwd and home directory.
    #[must_use]
    pub fn new(cwd: &'a Path, home: &'a Path) -> Self {
        Self { cwd, home }
    }

    /// Resolves a raw `@path` token body (everything after `@`) into an
    /// absolute path.
    ///
    /// - `~` or `~/...` → joined against `home`.
    /// - `/...` → used as-is (absolute).
    /// - anything else → joined against `cwd` (relative).
    #[must_use]
    pub fn resolve(&self, raw: &str) -> PathBuf {
        if raw == "~" {
            self.home.to_path_buf()
        } else if let Some(rest) = raw.strip_prefix("~/") {
            self.home.join(rest)
        } else if raw.starts_with('/') {
            PathBuf::from(raw)
        } else {
            self.cwd.join(raw)
        }
    }
}

/// A `@` followed by a path token at a word boundary (start of buffer or
/// preceded by space/newline). The path runs to the next whitespace.
///
/// Capture group 1 is the boundary (`^` or a space/newline); group 2 is the
/// raw path text (everything after `@` up to whitespace). `foo@path` does not
/// match because the char before `@` is not a boundary.
static AT_PATH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(^|[ \n])@([^\s]+)").expect("valid @path regex"));

/// Result of scanning text for `@path` image references.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScanResult {
    /// The text with each `@/abs/path` replaced by `(file:///abs/path)`.
    pub rewritten_text: String,
    /// The image attachments read from the referenced files, in order.
    pub attachments: Vec<Attachment>,
}

/// Scans `text` for `@path` tokens, resolves each against `ctx`, reads it as
/// an image, and rewrites the token to `(file:///resolved/abs/path)` form.
///
/// Files that do not exist, cannot be read, or have an unrecognized image
/// format are left as their literal `@path` token (no rewrite, no attachment).
/// This is a pure, fallible-per-file operation: a single bad path does not
/// abort the whole scan.
///
/// Path existence is confirmed but content validity is not — see module docs.
#[must_use]
pub fn scan_at_paths(text: &str, ctx: &PathResolveContext<'_>) -> ScanResult {
    let mut attachments: Vec<Attachment> = Vec::new();
    let rewritten_text = AT_PATH_RE
        .replace_all(text, |caps: &regex::Captures<'_>| {
            let boundary = &caps[1];
            let raw_path = &caps[2];
            let resolved = ctx.resolve(raw_path);
            match read_image_attachment(&resolved) {
                Some(attachment) => {
                    attachments.push(attachment);
                    // Use the resolved absolute path so terminals link to the real file.
                    format!("{boundary}(file://{})", resolved.display())
                }
                None => caps[0].to_owned(),
            }
        })
        .into_owned();
    ScanResult {
        rewritten_text,
        attachments,
    }
}

/// Reads the file at `path` and returns an image attachment if the bytes sniff
/// to a supported image media type. Returns `None` for missing files, read
/// errors, or unrecognized formats.
fn read_image_attachment(path: &Path) -> Option<Attachment> {
    let bytes = std::fs::read(path).ok()?;
    let media_type = sniff_media_type(&bytes)?;
    Some(Attachment::image(media_type, bytes))
}

/// Returns the MIME type for supported image magic-byte signatures, or `None`
/// if the bytes do not match a known image format.
///
/// Supports the formats vision models commonly accept: PNG, JPEG, GIF, WEBP.
#[must_use]
fn sniff_media_type(bytes: &[u8]) -> Option<&'static str> {
    // Order matters only for prefixes that are prefixes of others; none here overlap.
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12
        && bytes.starts_with(b"RIFF")
        && bytes.get(8..12) == Some(&b"WEBP"[..])
    {
        Some("image/webp")
    } else {
        None
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

    /// Context with arbitrary cwd/home — absolute paths ignore both, so this
    /// suffices for any test that doesn't exercise relative/tilde resolution.
    fn dummy_ctx() -> PathResolveContext<'static> {
        PathResolveContext::new(std::path::Path::new("/"), std::path::Path::new("/"))
    }

    /// Minimal valid PNG (1×1 transparent) for read-path tests.
    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // signature
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15,
        0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01,
        0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
        0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[rstest::rstest]
    fn scan_extracts_one_attachment_from_at_path() {
        // Given a temp png file and text referencing it.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("img.png");
        std::fs::write(&path, TINY_PNG).expect("write");
        let abs = path.to_string_lossy().into_owned();
        let text = format!("describe @{abs}");

        // When scanning.
        let result = scan_at_paths(&text, &dummy_ctx());

        // Then exactly one image attachment was extracted.
        assert_eq!(result.attachments.len(), 1);
        assert!(result.attachments[0].is_image());
    }

    #[rstest::rstest]
    fn scan_rewrites_at_path_to_file_uri() {
        // Given a temp png file referenced inline.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("img.png");
        std::fs::write(&path, TINY_PNG).expect("write");
        let abs = path.to_string_lossy().into_owned();
        let text = format!("describe @{abs}");

        // When scanning.
        let result = scan_at_paths(&text, &dummy_ctx());

        // Then the @path was rewritten to the file:// URI form.
        assert!(
            result.rewritten_text.contains(&format!("(file://{abs})")),
            "rewritten text should contain the file:// URI, got: {}",
            result.rewritten_text
        );
        assert!(
            !result.rewritten_text.contains(&format!("@{abs}")),
            "original @path token should be gone"
        );
    }

    #[rstest::rstest]
    fn scan_does_not_match_email_at() {
        // Given text containing an email-style @ not followed by a path.
        let text = "contact foo@bar.com please";

        // When scanning.
        let result = scan_at_paths(text, &dummy_ctx());

        // Then no attachments were extracted and text is unchanged.
        assert!(result.attachments.is_empty());
        assert_eq!(result.rewritten_text, text);
    }

    #[rstest::rstest]
    fn scan_leaves_missing_file_unchanged() {
        // Given a reference to a nonexistent path.
        let text = "see @/nonexistent/path.png here";

        // When scanning.
        let result = scan_at_paths(text, &dummy_ctx());

        // Then no attachment was added and the token is preserved.
        assert!(result.attachments.is_empty());
        assert_eq!(result.rewritten_text, text);
    }

    #[rstest::rstest]
    fn scan_leaves_non_image_file_unchanged() {
        // Given a temp file that is not an image.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, b"hello not an image").expect("write");
        let abs = path.to_string_lossy().into_owned();
        let text = format!("see @{abs}");

        // When scanning.
        let result = scan_at_paths(&text, &dummy_ctx());

        // Then no attachment was added and the token is preserved.
        assert!(result.attachments.is_empty());
        assert!(result.rewritten_text.contains(&format!("@{abs}")));
    }

    #[rstest::rstest]
    fn scan_at_path_at_start_of_string() {
        // Given a temp png referenced at the very start of the text.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("img.png");
        std::fs::write(&path, TINY_PNG).expect("write");
        let abs = path.to_string_lossy().into_owned();
        let text = format!("@{abs} what is this");

        // When scanning.
        let result = scan_at_paths(&text, &dummy_ctx());

        // Then the attachment was extracted even at string start.
        assert_eq!(result.attachments.len(), 1);
        assert!(result.rewritten_text.starts_with("(file://"));
    }

    #[rstest::rstest]
    fn scan_resolves_relative_path_against_cwd() {
        // Given a png in a temp dir used as the cwd.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("img.png");
        std::fs::write(&path, TINY_PNG).expect("write");
        let text = "describe @img.png here".to_owned();

        // When scanning with a context rooted at the temp dir.
        let ctx = PathResolveContext::new(dir.path(), std::path::Path::new("/"));
        let result = scan_at_paths(&text, &ctx);

        // Then exactly one image attachment was extracted.
        assert_eq!(result.attachments.len(), 1);
        assert!(result.attachments[0].is_image());
    }

    #[rstest::rstest]
    fn scan_resolves_tilde_path_against_home() {
        // Given a png under a temp dir treated as home.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("photo.png");
        std::fs::write(&path, TINY_PNG).expect("write");
        let text = "describe @~/photo.png here".to_owned();

        // When scanning with home pointing at the temp dir.
        let ctx = PathResolveContext::new(std::path::Path::new("/"), dir.path());
        let result = scan_at_paths(&text, &ctx);

        // Then exactly one image attachment was extracted.
        assert_eq!(result.attachments.len(), 1);
        assert!(result.attachments[0].is_image());
    }

    #[rstest::rstest]
    fn scan_tilde_alone_resolves_to_home() {
        // Given a png named directly as home (no subpath).
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("bare.png");
        std::fs::write(&path, TINY_PNG).expect("write");
        let text = "@~/bare.png".to_owned();

        // When scanning with home at the temp dir.
        let ctx = PathResolveContext::new(std::path::Path::new("/"), dir.path());
        let result = scan_at_paths(&text, &ctx);

        // Then the attachment was extracted.
        assert_eq!(result.attachments.len(), 1);
    }

    #[rstest::rstest]
    fn scan_space_in_filename_not_supported() {
        // Given a png whose name contains a space, and text that references
        // it by name. The @ token runs to the next whitespace, so `@my` is
        // the whole token — the space terminates it and the real file is
        // never read.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("my file.png");
        std::fs::write(&path, TINY_PNG).expect("write");
        let text = "describe @my file.png here".to_owned();

        // When scanning with cwd at the temp dir.
        let ctx = PathResolveContext::new(dir.path(), std::path::Path::new("/"));
        let result = scan_at_paths(&text, &ctx);

        // Then no attachment was extracted.
        assert!(result.attachments.is_empty());
    }

    #[rstest::rstest]
    fn scan_relative_path_descends_into_subdirectory() {
        // Given a png in a subdirectory of cwd.
        let dir = tempfile::tempdir().expect("temp dir");
        let subdir = dir.path().join("sub");
        std::fs::create_dir(&subdir).expect("mkdir");
        let path = subdir.join("img.png");
        std::fs::write(&path, TINY_PNG).expect("write");
        let text = "see @sub/img.png".to_owned();

        // When scanning with cwd at the temp dir root.
        let ctx = PathResolveContext::new(dir.path(), std::path::Path::new("/"));
        let result = scan_at_paths(&text, &ctx);

        // Then the attachment was extracted from the subdirectory.
        assert_eq!(result.attachments.len(), 1);
    }

    #[rstest::rstest]
    fn sniff_png_from_magic_bytes() {
        // Given PNG magic bytes.
        // Then the sniffer identifies it as image/png.
        assert_eq!(sniff_media_type(TINY_PNG), Some("image/png"));
    }

    #[rstest::rstest]
    fn sniff_jpeg_from_magic_bytes() {
        // Given JPEG magic bytes.
        // Then the sniffer identifies it as image/jpeg.
        assert_eq!(sniff_media_type(b"\xff\xd8\xff\xe0"), Some("image/jpeg"));
    }

    #[rstest::rstest]
    fn sniff_returns_none_for_unknown() {
        // Given non-image bytes.
        // Then the sniffer returns None.
        assert_eq!(sniff_media_type(b"plain text"), None);
    }
}
