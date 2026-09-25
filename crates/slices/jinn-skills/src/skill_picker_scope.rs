//! The skill picker's scope and route rows.
//!
//! The skill picker's scope is a dynamic slice scope, so the kernel has no
//! `Scope::PickerSkill` variant to keep in sync: the picker is a scope the
//! slice mints for itself.

use jinn_slices::SliceScopeId;

/// The skill picker's dynamic scope (input-capturing: the filter is text).
#[must_use]
pub fn skill_picker_scope() -> SliceScopeId {
    SliceScopeId::new("skills", "picker")
}
