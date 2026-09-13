//! Re-export of the shared [`LineInput`] text-editing primitive.
//!
//! The primitive moved to `jinn-slices` (kernel layer) so slice crates
//! can own input state without depending on `jinn-domain`.

pub use jinn_slices::line_input::LineInput;
