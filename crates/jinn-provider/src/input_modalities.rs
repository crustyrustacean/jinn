//! Input modalities a model accepts (text, image, and future audio/video).
//!
//! [`InputModalities`] is a small flags newtype over a private `u8`. All access
//! goes through the [`Modality`] enum; the bit layout is an implementation
//! detail that is never exposed. New modalities are added by extending the enum
//! and its bit table — no API changes ripple to call sites.

/// A single input modality a model may support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modality {
    /// Text input.
    Text,
    /// Image input (vision).
    Image,
}

impl Modality {
    /// The single-character glyph used in compact status displays.
    const fn glyph(self) -> char {
        match self {
            Modality::Text => 't',
            Modality::Image => 'i',
        }
    }

    /// The internal bit position for this modality.
    const fn bit(self) -> u8 {
        match self {
            Modality::Text => 1 << 0,
            Modality::Image => 1 << 1,
        }
    }
}

/// The set of input modalities a model supports.
///
/// Stored as a private `u8`; query and mutate exclusively through
/// [`Modality`]. Display order is fixed (text first) so the compact status
/// glyph is deterministic regardless of insertion order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct InputModalities(u8);

impl InputModalities {
    /// Text-only — every model accepts text.
    #[must_use]
    pub const fn text() -> Self {
        Self(Modality::Text.bit())
    }

    /// Returns `true` if `modality` is present.
    #[must_use]
    pub const fn contains(self, modality: Modality) -> bool {
        (self.0 & modality.bit()) != 0
    }

    /// Adds `modality` to the set if not already present.
    pub fn insert(&mut self, modality: Modality) {
        self.0 |= modality.bit();
    }

    /// Compact ordered display, e.g. `"ti"` (text then image).
    ///
    /// Always text-first. Empty when no modalities are set (only possible for a
    /// `Default`/serde-fallback value; callers that want to always show text
    /// should construct via [`InputModalities::text`]).
    #[must_use]
    pub fn display(self) -> String {
        let mut s = String::new();
        for modality in [Modality::Text, Modality::Image] {
            if self.contains(modality) {
                s.push(modality.glyph());
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::*;

    #[rstest::rstest]
    fn text_default_contains_only_text() {
        // Given text-only input modalities.
        let modalities = InputModalities::text();

        // Then text is present and image is absent, displaying "t".
        assert!(modalities.contains(Modality::Text));
        assert!(!modalities.contains(Modality::Image));
        assert_eq!(modalities.display(), "t");
    }

    #[test]
    fn insert_image_keeps_text() {
        // Given text-only modalities.
        let mut modalities = InputModalities::text();

        // When inserting image.
        modalities.insert(Modality::Image);

        // Then both text and image are present.
        assert!(modalities.contains(Modality::Text));
        assert!(modalities.contains(Modality::Image));
    }

    #[test]
    fn display_order_is_text_then_image_regardless_of_insert_order() {
        // Given modalities with image inserted before text.
        let mut modalities = InputModalities::text();
        modalities.insert(Modality::Image);

        // When displaying.
        let display = modalities.display();

        // Then the order is text-then-image ("ti"), not insertion order.
        assert_eq!(display, "ti");
    }

    #[test]
    fn serde_roundtrip_preserves_flags() {
        // Given modalities with both text and image set.
        let mut modalities = InputModalities::text();
        modalities.insert(Modality::Image);

        // When serializing to an integer and back.
        let value = serde_json::to_value(modalities).expect("serialize");
        let restored: InputModalities = serde_json::from_value(value).expect("deserialize");

        // Then both flags survive the round trip.
        assert!(restored.contains(Modality::Text));
        assert!(restored.contains(Modality::Image));
    }
}
