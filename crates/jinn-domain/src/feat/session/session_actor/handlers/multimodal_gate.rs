//! Vision-capability gate for multimodal sends.
//!
//! Before a user entry carrying image attachments is dispatched to the LLM,
//! the active model is checked against the models.dev reference data. A model
//! that is **not confirmed** vision-capable is blocked: instead of dispatching,
//! an explanatory `ChatEntryKind::Error` entry is pushed and the session returns
//! to `Idle`.
//!
//! Only a model positively identified as image-capable
//! (`supports_images() == Some(true)`) is allowed through. Unknown models
//! (`None` — not in the reference data) are blocked, not allowed: the user
//! maintains the reference data / model cache to mark a model capable.

use crate::feat::provider_infra::ModelsDevData;
use crate::protocol::{ChatEntry, ChatEntryKind};

/// Decides whether a user entry with attachments may be dispatched to the model.
///
/// Returns `None` only when the send is allowed — a text-only entry, or a model
/// confirmed image-capable by `models.dev`. Returns `Some(error_entry)` when the
/// entry carries attachments but the model is **not confirmed** image-capable:
/// either positively text-only (`Some(false)`) or unknown (`None`). The caller
/// pushes the error entry and skips dispatch.
///
/// `model_id` is the resolved model id (e.g. `"gpt-4o"`); `None` signals no
/// provider is configured, in which case attachments are allowed (the request
/// is a no-op dispatch anyway).
#[must_use]
pub fn attachment_gate(
    model_id: Option<&str>,
    entry: &ChatEntry,
    models_dev: &ModelsDevData,
) -> Option<ChatEntry> {
    let ChatEntryKind::User { attachments, .. } = &entry.kind else {
        return None;
    };
    if attachments.is_empty() {
        return None;
    }

    // No provider configured — nothing to gate (dispatch is a no-op).
    let model_id = model_id?;

    match models_dev.supports_images(model_id) {
        // Only a positively confirmed vision model is allowed through.
        Some(true) => None,
        // Some(false) → known text-only. None → unknown (not in reference data).
        // Both block: the user marks a model capable via models.dev / the cache.
        Some(false) | None => Some(blocked_error_entry(model_id)),
    }
}

/// Builds the `Error` entry shown when a model is not confirmed vision-capable.
///
/// Names the model and points the user at models.dev / the model cache so they
/// can mark it image-capable.
fn blocked_error_entry(model_id: &str) -> ChatEntry {
    ChatEntry::error(format!(
        "{model_id} is not confirmed to support image input, so this attachment was not sent. Switch to a vision-capable model, or mark it image-capable in models.dev / the model cache."
    ))
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
    use jinn_provider::Attachment;

    fn entry_with_image() -> ChatEntry {
        let mut entry = ChatEntry::user("describe this");
        if let ChatEntryKind::User { attachments, .. } = &mut entry.kind {
            attachments.push(Attachment::image("image/png", vec![1, 2, 3]));
        }
        entry
    }

    #[test]
    fn allows_text_only_entry_regardless_of_model() {
        // Given a text-only entry and a non-vision model.
        let models_dev = ModelsDevData::new();
        let entry = ChatEntry::user("hello");

        // When gating.
        // Then None is returned (no block).
        assert!(attachment_gate(Some("gpt-3.5-turbo"), &entry, &models_dev).is_none());
    }

    #[test]
    fn blocks_attachment_on_known_text_only_model() {
        // Given an image entry and a model known to lack image support.
        let mut models_dev = ModelsDevData::new();
        models_dev
            .image_support
            .insert("gpt-3.5-turbo".to_owned(), false);
        let entry = entry_with_image();

        // When gating.
        let blocked = attachment_gate(Some("gpt-3.5-turbo"), &entry, &models_dev);

        // Then an error entry is returned.
        assert!(blocked.is_some(), "expected a block error entry");
    }

    #[test]
    fn allows_attachment_on_vision_model() {
        // Given an image entry and a vision-capable model.
        let mut models_dev = ModelsDevData::new();
        models_dev.image_support.insert("gpt-4o".to_owned(), true);
        let entry = entry_with_image();

        // When gating.
        // Then None is returned (allowed).
        assert!(attachment_gate(Some("gpt-4o"), &entry, &models_dev).is_none());
    }

    #[test]
    fn blocks_attachment_on_unknown_model() {
        // Given an image entry and a model not in the reference data.
        let models_dev = ModelsDevData::new();
        let entry = entry_with_image();

        // When gating.
        let blocked = attachment_gate(Some("my-custom-llama"), &entry, &models_dev);

        // Then an error entry is returned (unknown models block, not allowed).
        assert!(
            blocked.is_some(),
            "unknown models must be blocked unless positively confirmed vision-capable"
        );
    }

    #[test]
    fn blocked_error_entry_names_model_and_points_at_models_dev() {
        // Given an unknown model id.
        let models_dev = ModelsDevData::new();
        let entry = entry_with_image();

        // When gating.
        let blocked =
            attachment_gate(Some("my-custom-llama"), &entry, &models_dev).expect("blocked");

        // Then the Error entry text names the model id AND points at models.dev / cache.
        let text = match &blocked.kind {
            ChatEntryKind::Error(t) => t.clone(),
            _ => panic!("expected an Error entry"),
        };
        assert!(
            text.contains("my-custom-llama"),
            "must name the model id: {text}"
        );
        assert!(
            text.contains("models.dev"),
            "must point the user at models.dev / the cache: {text}"
        );
    }

    #[test]
    fn allows_attachment_when_no_provider_configured() {
        // Given an image entry and no configured model.
        let models_dev = ModelsDevData::new();
        let entry = entry_with_image();

        // When gating.
        // Then None is returned (no provider → no-op dispatch).
        assert!(attachment_gate(None, &entry, &models_dev).is_none());
    }
}
