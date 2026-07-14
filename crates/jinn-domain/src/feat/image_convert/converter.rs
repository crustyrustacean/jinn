//! Image conversion to PNG via the ImageMagick CLI.
//!
//! Non-native image formats (HEIC, AVIF, TIFF, BMP, …) that the native
//! magic-byte sniffer rejects are handed to [`ImageMagickConverter`], which
//! shells out to `magick <input> png:-` (or `convert <input> png:-` for v6)
//! and captures the transcoded PNG bytes from stdout. Conversion is invoked
//! inside `tokio::task::spawn_blocking` by the session actor, keeping the
//! blocking process spawn off the async runtime.
//!
//! If no ImageMagick binary is on `PATH` at construction time, the converter
//! reports `is_available() == false`; the actor then surfaces a
//! `ChatEntryKind::Error` rather than silently dropping the attachment.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use derive_more::Debug;
use error_stack::{Report, ResultExt};
use wherror::Error;

use super::binary_locator::{ImageMagickLocator, SystemImageMagickLocator};

/// Conversion failure. The context attached via `.attach(...)` identifies the
/// input path and the phase that failed (binary missing, non-zero exit, …).
#[derive(Debug, Error)]
#[error(debug)]
pub struct ImageConversionError;

/// Converts a non-native image file at `input` to PNG bytes.
///
/// Production uses [`ImageMagickConverter`]; tests inject fakes that return
/// canned bytes or simulate failure. The trait is synchronous because callers
/// run it inside `spawn_blocking`.
pub trait ImageConverter: Send + Sync {
    /// Debug name of the backend (e.g. `"ImageMagick"`).
    fn name(&self) -> &'static str;

    /// Whether a converter backend was available at construction time.
    fn is_available(&self) -> bool;

    /// Convert the image at `input` to PNG bytes. Returns the transcoded bytes
    /// (validated to start with the PNG magic signature) on success.
    ///
    /// # Errors
    ///
    /// Returns `Report<ImageConversionError>` if the binary is unavailable,
    /// the process exits non-zero, or the captured stdout is not a valid PNG.
    fn convert_to_png(&self, input: &Path) -> Result<Vec<u8>, Report<ImageConversionError>>;
}

/// Production converter backed by the ImageMagick CLI.
///
/// Holds the resolved binary path (probed once at construction via the
/// locator). When `None`, `is_available()` is false and `convert_to_png`
/// returns an error immediately without spawning a process.
pub struct ImageMagickConverter {
    binary: Option<PathBuf>,
}

impl ImageMagickConverter {
    /// Probe `PATH` for ImageMagick using the system locator.
    #[must_use]
    pub fn new() -> Self {
        Self::with_locator(&SystemImageMagickLocator)
    }

    /// Construct with an explicit locator (for tests or alternative search
    /// orders). The locator is consulted exactly once here.
    #[must_use]
    pub fn with_locator<L: ImageMagickLocator>(locator: &L) -> Self {
        Self {
            binary: locator.find_binary(),
        }
    }
}

impl Default for ImageMagickConverter {
    fn default() -> Self {
        Self::new()
    }
}

/// PNG magic signature: `\x89PNG\r\n\x1a\n`.
const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

impl ImageConverter for ImageMagickConverter {
    fn name(&self) -> &'static str {
        "ImageMagick"
    }

    fn is_available(&self) -> bool {
        self.binary.is_some()
    }

    fn convert_to_png(&self, input: &Path) -> Result<Vec<u8>, Report<ImageConversionError>> {
        let binary = self.binary.as_ref().ok_or_else(|| {
            Report::new(ImageConversionError)
                .change_context(ImageConversionError)
                .attach("ImageMagick (magick/convert) not found on PATH")
        })?;

        let output = std::process::Command::new(binary)
            .arg(input)
            .arg("png:-") // write PNG to stdout
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .change_context(ImageConversionError)
            .attach(input.to_string_lossy().to_string())
            .attach("failed to spawn ImageMagick")?;

        if !output.status.success() {
            return Err(Report::new(ImageConversionError)
                .change_context(ImageConversionError)
                .attach(input.to_string_lossy().to_string())
                .attach(format!(
                    "ImageMagick exited with code {:?}",
                    output.status.code()
                )));
        }

        if !output.stdout.starts_with(PNG_MAGIC) {
            return Err(Report::new(ImageConversionError)
                .change_context(ImageConversionError)
                .attach(input.to_string_lossy().to_string())
                .attach("ImageMagick output is not a valid PNG"));
        }

        Ok(output.stdout)
    }
}

/// Shared, cloneable service wrapper around the converter trait object, per
/// the AGENTS.md service-wrapper pattern.
#[derive(Debug, Clone)]
pub struct ImageConverterService {
    #[debug("ImageConverter<{}>", self.backend.name())]
    backend: Arc<dyn ImageConverter>,
}

impl ImageConverterService {
    #[must_use]
    pub fn new(backend: Arc<dyn ImageConverter>) -> Self {
        Self { backend }
    }

    /// Construct a production service backed by [`ImageMagickConverter`].
    #[must_use]
    pub fn system() -> Self {
        Self::new(Arc::new(ImageMagickConverter::new()))
    }

    /// Construct a test service backed by a converter that reports itself
    /// unavailable (no ImageMagick). Used by test helpers that don't exercise
    /// the conversion path.
    #[must_use]
    pub fn unavailable() -> Self {
        Self::new(Arc::new(crate::feat::image_convert::UnavailableConverter))
    }

    #[must_use]
    pub fn name(&self) -> &'static str {
        self.backend.name()
    }

    #[must_use]
    pub fn is_available(&self) -> bool {
        self.backend.is_available()
    }

    /// See [`ImageConverter::convert_to_png`].
    ///
    /// # Errors
    ///
    /// Propagates the converter's error.
    pub fn convert_to_png(&self, input: &Path) -> Result<Vec<u8>, Report<ImageConversionError>> {
        self.backend.convert_to_png(input)
    }
}

/// A converter that reports itself unavailable — used by test helpers that
/// don't exercise the conversion path. Conversion attempts always error.
pub struct UnavailableConverter;

impl ImageConverter for UnavailableConverter {
    fn name(&self) -> &'static str {
        "Unavailable"
    }
    fn convert_to_png(&self, _input: &Path) -> Result<Vec<u8>, Report<ImageConversionError>> {
        Err(Report::new(ImageConversionError))
    }
    fn is_available(&self) -> bool {
        false
    }
}
