//! Composition-side picker registry — where specs are built and registered.
//!
//! This module is the adapter between the kernel's legacy `PickerKind` world
//! and the generic `jinn-picker` spec world: specs register here at
//! composition time, and [`jinn_picker::spec_id_for_kind`] maps the static
//! per-kind scopes/intents onto the registry until every picker has migrated
//! to its owning slice.
//!
//! When the last picker migrates, the kind→id mapping is deleted;
//! `PickerRegistry::ids` then *is* the picker vocabulary.

use jinn_picker::PickerRegistry;

/// Builds the picker registry: every migrated picker registers its spec here
/// once at composition.
///
/// The kernel cannot call this — it must not depend on this crate — so
/// kernel-side entry writers wrap their items through
/// [`jinn_picker::make_items_with_hooks`] with the same hooks the
/// corresponding spec declares.
#[must_use]
pub fn build_picker_registry() -> PickerRegistry {
    let mut registry = PickerRegistry::new();
    registry.register(crate::persona_spec::persona_spec());
    registry.register(crate::skill_spec::skill_spec());
    registry.register(crate::theme_spec::theme_spec());
    registry.register(crate::tool_spec::tool_spec());
    registry.register(crate::mcp_server_spec::mcp_server_spec());
    registry.register(crate::session_lifecycle_spec::session_lifecycle_spec());
    registry.register(crate::reasoning_effort_spec::reasoning_effort_spec());
    registry.register(crate::task_list_spec::task_list_spec());
    registry.register(crate::session_spec::session_spec());
    registry.register(crate::provider_spec::provider_spec());
    registry.register(crate::endpoint_spec::endpoint_spec());
    registry.register(crate::project_spec::project_spec());
    registry
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test module, panics are acceptable")]
    use super::*;
    use jinn_slices::picker_kind::PickerKind;

    #[rstest::rstest]
    #[test]
    fn migrated_kinds_map_onto_registered_specs_and_vice_versa() {
        // Given the picker registry and the kind→id adapter.
        let registry = build_picker_registry();
        let migrated = [
            PickerKind::Persona,
            PickerKind::Skill,
            PickerKind::Theme,
            PickerKind::Tool,
            PickerKind::McpServer,
            PickerKind::SessionLifecycle,
            PickerKind::ReasoningEffort,
            PickerKind::TaskList,
            PickerKind::Session,
            PickerKind::Provider,
            PickerKind::Endpoint,
            PickerKind::Project,
        ];

        // When mapping each migrated kind and listing registered ids.
        let mapped_ids: Vec<&str> = migrated
            .iter()
            .filter_map(jinn_picker::spec_id_for_kind)
            .collect();
        let mut registered_ids = registry.ids();
        registered_ids.sort_unstable();

        // Then the two sets are exactly equal — a kind with a spec id but
        // no registered spec (or the reverse) is a wiring bug.
        let mut expected = mapped_ids.clone();
        expected.sort_unstable();
        assert_eq!(registered_ids, expected);
        assert_eq!(mapped_ids.len(), migrated.len());
    }
}
