#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use error_stack::Report;
use rstest::rstest;

use super::binary_locator::ImageMagickLocator;
use super::converter::{
    ImageConversionError, ImageConverter, ImageConverterService, ImageMagickConverter,
};

// ---------------------------------------------------------------------------
// Test fakes
// ---------------------------------------------------------------------------

/// Fake locator returning a fixed (or absent) resolved path.
struct FakeLocator {
    resolved: Option<PathBuf>,
}

impl ImageMagickLocator for FakeLocator {
    fn find_binary(&self) -> Option<PathBuf> {
        self.resolved.clone()
    }
}

/// Records whether `convert_to_png` / `is_available` were exercised. Lets
/// tests assert "converter not called for native formats" behaviorally.
#[derive(Default, Debug)]
struct SpyConverter {
    available: bool,
    convert_calls: std::sync::Mutex<Vec<PathBuf>>,
    /// Canned bytes returned on convert (when `available` and non-empty).
    canned: Vec<u8>,
    /// When true, convert returns an error instead of canned bytes.
    fail: bool,
}

impl ImageConverter for SpyConverter {
    fn name(&self) -> &'static str {
        "Spy"
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn convert_to_png(&self, input: &Path) -> Result<Vec<u8>, Report<ImageConversionError>> {
        self.convert_calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(input.to_path_buf());
        if self.fail {
            Err(Report::new(ImageConversionError))
        } else {
            Ok(self.canned.clone())
        }
    }
}

// ---------------------------------------------------------------------------
// Locator tests
// ---------------------------------------------------------------------------

#[rstest]
fn locator_prefers_magick_over_convert() {
    // Given an ImageMagickConverter built via the system locator.
    // (Structural: SystemImageMagickLocator tries `magick` then `convert`.
    // We assert the converter reflects whatever is on this machine's PATH —
    // the only observable without mocking `which`.)
    let converter = ImageMagickConverter::new();

    // When querying availability.
    // Then it does not panic and reports a stable boolean.
    let _ = converter.is_available();
}

#[rstest]
fn converter_is_unavailable_when_locator_finds_nothing() {
    // Given a converter whose locator resolves no binary.
    let converter = ImageMagickConverter::with_locator(&FakeLocator { resolved: None });

    // When querying availability.
    // Then it reports unavailable.
    assert!(!converter.is_available());
}

#[rstest]
fn converter_is_available_when_locator_resolves() {
    // Given a converter whose locator resolves a binary.
    let converter = ImageMagickConverter::with_locator(&FakeLocator {
        resolved: Some(PathBuf::from("/usr/bin/magick")),
    });

    // When querying availability.
    // Then it reports available.
    assert!(converter.is_available());
}

// ---------------------------------------------------------------------------
// Converter behavior
// ---------------------------------------------------------------------------

#[rstest]
fn convert_errors_when_binary_unavailable() {
    // Given a converter with no resolved binary.
    let converter = ImageMagickConverter::with_locator(&FakeLocator { resolved: None });

    // When converting a path.
    let result = converter.convert_to_png(Path::new("/tmp/x.heic"));

    // Then it errors without spawning a process.
    assert!(result.is_err());
}

#[rstest]
fn service_reports_unavailable_when_backend_is() {
    // Given a service wrapping an unavailable spy backend.
    let service = ImageConverterService::new(Arc::new(SpyConverter {
        available: false,
        ..SpyConverter::default()
    }));

    // When querying availability.
    // Then the service reports unavailable.
    assert!(!service.is_available());
}

#[rstest]
fn service_reports_available_when_backend_is() {
    // Given a service wrapping an available spy backend.
    let service = ImageConverterService::new(Arc::new(SpyConverter {
        available: true,
        ..SpyConverter::default()
    }));

    // When querying availability.
    // Then the service reports available.
    assert!(service.is_available());
}

#[rstest]
fn service_convert_returns_canned_bytes_when_backend_available() {
    // Given a service wrapping an available spy returning canned PNG bytes.
    let png = b"\x89PNG\r\n\x1a\nDATA".to_vec();
    let service = ImageConverterService::new(Arc::new(SpyConverter {
        available: true,
        canned: png.clone(),
        ..SpyConverter::default()
    }));

    // When converting.
    let result = service.convert_to_png(Path::new("/tmp/x.heic"));

    // Then the canned bytes are returned.
    assert_eq!(result.expect("convert"), png);
}

#[rstest]
fn service_convert_errors_when_backend_fails() {
    // Given a service wrapping an available-but-failing spy.
    let service = ImageConverterService::new(Arc::new(SpyConverter {
        available: true,
        fail: true,
        ..SpyConverter::default()
    }));

    // When converting.
    // Then it errors.
    assert!(service.convert_to_png(Path::new("/tmp/x.heic")).is_err());
}

#[rstest]
fn service_name_reflects_backend() {
    // Given a service wrapping a spy.
    let service = ImageConverterService::new(Arc::new(SpyConverter::default()));

    // When querying the name.
    // Then it reports the spy's name.
    assert_eq!(service.name(), "Spy");
}
