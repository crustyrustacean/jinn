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

/// A registry holding exactly one picker spec.
///
/// Kernel dispatch paths that resolve a single known picker (a sidebar row
/// that opens the task-list picker, say) can build just that spec instead of
/// the full fan-out. Keeps each caller linked only to the picker it names.
#[must_use]
pub fn single_spec_registry<T>(spec: jinn_picker::PickerSpec<T>) -> PickerRegistry
where
    T: jinn_selection_widget::TreeItem + std::fmt::Debug + Send + Sync + 'static,
{
    let mut registry = PickerRegistry::new();
    registry.register(spec);
    registry
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test module, panics are acceptable")]
    use super::*;
    use jinn_slices::picker_kind::PickerKind;

    #[rstest::rstest]
    #[test]
    fn single_spec_registry_resolves_only_the_named_spec() {
        // Given a registry holding just the persona spec.
        let registry = single_spec_registry(crate::persona_spec::persona_spec());

        // When looking specs up by id.
        let persona = registry.get(jinn_picker::PERSONA_ID);
        let other = registry.get(jinn_picker::PROJECT_ID);

        // Then only the named spec resolves — the caller links to no other picker.
        assert!(persona.is_some());
        assert!(other.is_none());
    }

    #[rstest::rstest]
    #[test]
    fn single_spec_registry_make_items_matches_the_full_registry() {
        // Given a one-spec registry and the full fan-out registry.
        let one = single_spec_registry(crate::persona_spec::persona_spec());
        let all = build_picker_registry();
        let entries = || {
            vec![jinn_persona_msg::PersonaEntry {
                name: "coder".to_owned(),
                description: "code helper".to_owned(),
                is_active: false,
                theme: jinn_theme::default_theme(),
            }]
        };

        // When wrapping identical entries through each.
        let via_one =
            one.make_items::<jinn_persona_msg::PersonaEntry>(jinn_picker::PERSONA_ID, entries());
        let via_all =
            all.make_items::<jinn_persona_msg::PersonaEntry>(jinn_picker::PERSONA_ID, entries());

        // Then the wrapped items are identical — the seam loses nothing.
        assert_eq!(via_one.is_some(), via_all.is_some());
        assert_eq!(
            via_one.as_ref().and_then(|items| items
                .first()
                .map(jinn_selection_widget::PickerItem::display_label)),
            via_all.as_ref().and_then(|items| items
                .first()
                .map(jinn_selection_widget::PickerItem::display_label)),
        );
    }

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
