//! Blocking image-attachment resolution: read + classify + convert.
//!
//! These free functions run inside `spawn_blocking` (off the async runtime) and
//! turn a list of resolved `@path` file paths into [`Attachment`]s:
//!
//! 1. Read the file bytes.
//! 2. Classify via [`classify_image_bytes`] as native / needs-conversion /
//!    not-an-image.
//! 3. Native formats (PNG/JPEG/GIF/WEBP) attach directly; non-native
//!    recognizable images are transcoded to PNG via the
//!    [`ImageConverterService`] (ImageMagick); anything else is an error.
//!
//! On any failure the whole resolution fails — the actor surfaces a single
//! `Error` entry rather than a partial attachment set.

use std::path::Path;

use error_stack::{Report, ResultExt};

use crate::feat::context::prompt_template::{ImageKind, classify_image_bytes};
use crate::feat::image_convert::ImageConverterService;
use jinn_provider::Attachment;

/// Errors that can occur while resolving `@path` image attachments.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub struct ImageResolveError;

/// Reads, classifies, and (if needed) converts each path into an
/// [`Attachment`]. Returns all attachments on success, or the first failure as
/// a [`Report<ImageResolveError>`].
///
/// This is a blocking function — callers must run it inside `spawn_blocking`.
pub(super) fn resolve_attachments_blocking(
    paths: &[std::path::PathBuf],
    converter: &ImageConverterService,
) -> Result<Vec<Attachment>, Report<ImageResolveError>> {
    let mut attachments = Vec::with_capacity(paths.len());
    for path in paths {
        let attachment = resolve_one_blocking(path, converter)?;
        attachments.push(attachment);
    }
    Ok(attachments)
}

/// Resolves a single path to an attachment.
fn resolve_one_blocking(
    path: &Path,
    converter: &ImageConverterService,
) -> Result<Attachment, Report<ImageResolveError>> {
    let bytes = std::fs::read(path)
        .change_context(ImageResolveError)
        .attach(path.to_string_lossy().to_string())
        .attach("failed to read image file")?;
    match classify_image_bytes(&bytes) {
        ImageKind::Native { media_type } => Ok(Attachment::image(media_type, bytes)),
        ImageKind::NeedsConversion => convert_via_imagemagick(path, converter),
        ImageKind::NotAnImage => Err(Report::new(ImageResolveError)
            .attach(path.to_string_lossy().to_string())
            .attach("file is not a recognized image format")),
    }
}

/// Transcodes a non-native image to PNG via ImageMagick. Errors when the
/// converter is unavailable (ImageMagick not found) or the conversion fails
/// (non-zero exit, invalid output).
fn convert_via_imagemagick(
    path: &Path,
    converter: &ImageConverterService,
) -> Result<Attachment, Report<ImageResolveError>> {
    let png_bytes = converter
        .convert_to_png(path)
        .change_context(ImageResolveError)
        .attach(path.to_string_lossy().to_string())?;
    Ok(Attachment::image("image/png", png_bytes))
}

/// Renders a resolution-failure [`Report`] into a user-facing message.
///
/// Joins the error's context chain (the top-level message plus attached
/// context frames) into a single string.
#[must_use]
pub(super) fn format_attachment_error(report: &Report<ImageResolveError>) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("Could not attach image: ");
    // The Report's Display already includes the error and all attachments.
    let _ = write!(out, "{report:}");
    out
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use error_stack::Report;

    use super::*;
    use crate::feat::image_convert::{ImageConversionError, ImageConverter};

    /// A fake converter that returns canned PNG bytes or fails, recording its
    /// calls so tests can assert conversion was/wasn't attempted.
    #[derive(Default)]
    struct FakeConverter {
        available: bool,
        fail: bool,
        calls: Mutex<Vec<PathBuf>>,
    }

    impl ImageConverter for FakeConverter {
        fn name(&self) -> &'static str {
            "Fake"
        }
        fn convert_to_png(&self, input: &Path) -> Result<Vec<u8>, Report<ImageConversionError>> {
            self.calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(input.to_path_buf());
            if !self.available {
                return Err(Report::new(ImageConversionError));
            }
            if self.fail {
                Err(Report::new(ImageConversionError))
            } else {
                Ok(b"\x89PNG\r\n\x1a\nfake".to_vec())
            }
        }
        fn is_available(&self) -> bool {
            self.available
        }
    }

    fn service(available: bool, fail: bool) -> ImageConverterService {
        ImageConverterService::new(Arc::new(FakeConverter {
            available,
            fail,
            calls: Mutex::new(Vec::new()),
        }))
    }

    fn png_bytes() -> Vec<u8> {
        b"\x89PNG\r\n\x1a\nbody".to_vec()
    }

    #[test]
    fn native_png_attaches_without_conversion() {
        // Given a native PNG file.
        let tmp = tempfile::tempdir().expect("tmp");
        let path = tmp.path().join("native.png");
        std::fs::write(&path, png_bytes()).expect("write");
        let converter = service(true, false);

        // When resolving.
        let result = resolve_attachments_blocking(std::slice::from_ref(&path), &converter);

        // Then exactly one image/png attachment is returned.
        let attachments = result.expect("resolve");
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].media_type(), "image/png");
    }

    #[test]
    fn non_native_image_is_converted_when_converter_available() {
        // Given a recognizable-but-non-native image (HEIC magic bytes).
        let tmp = tempfile::tempdir().expect("tmp");
        let path = tmp.path().join("photo.heic");
        // ftyp box: [size][ftyp][brand=heic]
        let mut heic = vec![0x00, 0x00, 0x00, 0x08];
        heic.extend_from_slice(b"ftyp");
        heic.extend_from_slice(b"heic");
        heic.extend_from_slice(b"payload");
        std::fs::write(&path, &heic).expect("write");
        let converter = service(true, false);

        // When resolving.
        let result = resolve_attachments_blocking(std::slice::from_ref(&path), &converter);

        // Then a converted image/png attachment is returned.
        let attachments = result.expect("resolve");
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].media_type(), "image/png");
    }

    #[test]
    fn non_native_image_errors_when_converter_unavailable() {
        // Given a non-native image and an unavailable converter.
        let tmp = tempfile::tempdir().expect("tmp");
        let path = tmp.path().join("photo.heic");
        let mut heic = vec![0x00, 0x00, 0x00, 0x08];
        heic.extend_from_slice(b"ftypheicpayload");
        std::fs::write(&path, &heic).expect("write");
        let converter = service(false, false);

        // When resolving.
        let result = resolve_attachments_blocking(std::slice::from_ref(&path), &converter);

        // Then it errors.
        assert!(result.is_err());
    }

    #[test]
    fn non_native_image_errors_when_conversion_fails() {
        // Given a non-native image and a failing converter.
        let tmp = tempfile::tempdir().expect("tmp");
        let path = tmp.path().join("photo.heic");
        let mut heic = vec![0x00, 0x00, 0x00, 0x08];
        heic.extend_from_slice(b"ftypheicpayload");
        std::fs::write(&path, &heic).expect("write");
        let converter = service(true, true);

        // When resolving.
        let result = resolve_attachments_blocking(&[path], &converter);

        // Then it errors.
        assert!(result.is_err());
    }

    #[test]
    fn not_an_image_errors() {
        // Given a non-image file.
        let tmp = tempfile::tempdir().expect("tmp");
        let path = tmp.path().join("notes.txt");
        std::fs::write(&path, b"hello world").expect("write");
        let converter = service(true, false);

        // When resolving.
        let result = resolve_attachments_blocking(&[path], &converter);

        // Then it errors.
        assert!(result.is_err());
    }

    #[test]
    fn missing_file_errors() {
        // Given a path to a nonexistent file.
        let converter = service(true, false);

        // When resolving.
        let result =
            resolve_attachments_blocking(&[PathBuf::from("/nonexistent/x.png")], &converter);

        // Then it errors.
        assert!(result.is_err());
    }

    #[test]
    fn mixed_native_and_non_native_all_resolve() {
        // Given one native PNG and one non-native HEIC.
        let tmp = tempfile::tempdir().expect("tmp");
        let png_path = tmp.path().join("a.png");
        std::fs::write(&png_path, png_bytes()).expect("write");
        let heic_path = tmp.path().join("b.heic");
        let mut heic = vec![0x00, 0x00, 0x00, 0x08];
        heic.extend_from_slice(b"ftypheicpayload");
        std::fs::write(&heic_path, &heic).expect("write");
        let converter = service(true, false);

        // When resolving both.
        let result = resolve_attachments_blocking(&[png_path, heic_path], &converter);

        // Then two image/png attachments are returned.
        let attachments = result.expect("resolve");
        assert_eq!(attachments.len(), 2);
        assert_eq!(attachments[0].media_type(), "image/png");
        assert_eq!(attachments[1].media_type(), "image/png");
    }
}
