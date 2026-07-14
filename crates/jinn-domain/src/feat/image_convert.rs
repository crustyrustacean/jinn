//! Image conversion for non-native `@path` attachments.
//!
//! Native image formats (PNG/JPEG/GIF/WEBP) attach directly without
//! conversion. Other image formats (HEIC, AVIF, TIFF, BMP, …) are
//! transcoded to PNG via the ImageMagick CLI by [`ImageConverterService`],
//! invoked from the session actor inside `spawn_blocking` so the blocking
//! process spawn never stalls the async runtime.
//!
//! Binary discovery mirrors the browser-binary resolver: a testable
//! [`binary_locator::ImageMagickLocator`] seam with a production
//! [`binary_locator::SystemImageMagickLocator`] that probes `PATH`.

mod binary_locator;
mod converter;

pub use binary_locator::{ImageMagickLocator, SystemImageMagickLocator};
pub use converter::{
    ImageConversionError, ImageConverter, ImageConverterService, ImageMagickConverter,
    UnavailableConverter,
};

#[cfg(test)]
mod tests;
