//! Vision-capability gate for multimodal sends.
//!
//! Before a user entry carrying image attachments is dispatched to the LLM,
//! the active model is checked against the models.dev reference data. A model
//! known to lack image support (`supports_images() == Some(false)`) is blocked:
//! instead of dispatching, an explanatory `ChatEntryKind::Error` entry is pushed
//! and the session returns to `Idle`.
//!
//! Unknown models (`supports_images() == None`) are allowed through — the gate
//! only rejects models it can positively identify as text-only.

use crate::feat::provider_infra::ModelsDevData;
use crate::protocol::{ChatEntry, ChatEntryKind};

/// Decides whether a user entry with attachments may be dispatched to the model.
///
/// Returns `None` when the send is allowed (text-only entry, vision-capable
/// model, or an unknown model we cannot positively rule out). Returns
/// `Some(error_entry)` when the entry carries attachments but the model is
/// known to lack image support — the caller pushes the error entry and skips
/// dispatch.
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
        Some(false) => Some(blocked_error_entry(model_id)),
        // Some(true) → vision model, allow. None → unknown model, allow.
        Some(true) | None => None,
    }
}

/// Builds the `Error` entry shown when a non-vision model is blocked.
fn blocked_error_entry(model_id: &str) -> ChatEntry {
    ChatEntry::error(format!(
        "{model_id} does not support image input. Switch to a vision-capable model to send attachments."
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
    fn allows_attachment_on_unknown_model() {
        // Given an image entry and a model not in the reference data.
        let models_dev = ModelsDevData::new();
        let entry = entry_with_image();

        // When gating.
        // Then None is returned (unknown models are allowed).
        assert!(
            attachment_gate(Some("my-custom-llama"), &entry, &models_dev).is_none(),
            "unknown models must not be blocked"
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
