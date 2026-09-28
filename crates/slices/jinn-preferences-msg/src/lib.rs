//! The preferences slice's shared cell vocabulary.
//!
//! The pruner accumulation threshold popup's editable state and its slot
//! key live here rather than in the slice crate so the shared cell
//! catalog can register the slot without depending on
//! `jinn-preferences`, which depends on the kernel. The slice
//! re-exports both so its own modules keep reading them from one path.

use jinn_slices::{LineInput, SliceScopeId, SlotKey};

/// The popup's dynamic input-capturing scope.
#[must_use]
pub fn pruner_accumulation_scope() -> SliceScopeId {
    SliceScopeId::new("preferences", "pruner_accumulation_input")
}

/// The slot containing the popup's editable threshold state.
#[must_use]
pub fn pruner_accumulation_slot() -> SlotKey {
    SlotKey::builtin("preferences", "pruner_accumulation_input")
}

/// The editable threshold and cursor for the pruner accumulation popup.
#[derive(Debug, Clone, Default)]
pub struct PrunerAccumulationInputState {
    /// The decimal threshold text and byte-offset cursor.
    pub text: LineInput,
}
