//! The skills slice — implementation services and the skills picker.
//!
//! Owns YAML frontmatter parsing, directory scanning, prompt formatting, the
//! loaded-skill label vocabulary, and the skills picker itself. Portable skill
//! values and crossing contracts live in `jinn-skills-msg`.
//!
//! The picker is fully slice-owned: its state is a cell, its keys are route
//! rows attached below, and its renderer reads the cell. It reaches session
//! state through the same `as_any_mut` seam the sidebar and terminal slices use,
//! so the kernel holds no skill-picker scope, keybind, or identifier.

pub mod format;
pub mod frontmatter;
pub mod scan;
pub mod skill;
pub mod skill_picker_actions;
pub mod skill_picker_reload;
pub mod skill_picker_render;
pub mod skill_picker_republisher_actor;
pub mod skill_picker_routes;
pub mod skill_picker_scope;
pub mod skill_picker_viewport;
pub mod skill_preview;

pub use format::format_skills_for_prompt;
pub use jinn_skills_msg::SKILL_CONTENT_PREFIX;
pub use jinn_skills_msg::SKILL_ICON;
pub use jinn_skills_msg::SkillPreviewCache;
pub use jinn_skills_msg::loaded_skill_summary_label;
pub use jinn_skills_msg::parse_loaded_skill_name;
pub use jinn_skills_msg::{Skill, SkillFrontmatter, SkillSource};
pub use scan::scan_skills;
pub use skill_picker_actions::{cancel, confirm, highlighted_name, open, toggle_highlighted};
pub use skill_picker_reload::{build_skill_entries, reload_skill_picker};
pub use skill_picker_render::{render_skill_picker, skill_picker_overlay_rect};
pub use skill_picker_routes::{SKILL_PICKER_BINDINGS, attach_skill_picker_rows};
pub use skill_picker_routes::{register_skill_picker_input_hook, republish_from_discovery};
pub use skill_picker_scope::skill_picker_scope;
pub use skill_preview::render_skill_preview;

/// Activates the slice: registers the picker's overlay, attaches its route
/// rows, and spawns its republisher actor.
///
/// The picker's cell is not minted here: the shared cell catalog
/// (`jinn_cell_catalog::register_all_cells`) registers every slice cell in
/// one place, before any slice activates, so this resolves the handle the
/// same way every other consumer does.
///
/// # Panics
///
/// Panics if the catalog has not run — the overlay would resolve no state
/// and render nothing, and the input hook would swallow keystrokes.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate(host: &mut jinn_slices::SliceHost<'_, jinn_slices::RenderFacts>) {
    let cell = host
        .slices()
        .reader::<jinn_skills_msg::SkillPickerState>(&jinn_skills_msg::skill_picker_slot())
        .expect("the cell catalog registers the skill picker slot before any slice activates");
    let scope = skill_picker_scope();
    host.register_overlay(
        scope.clone(),
        std::sync::Arc::new(skill_picker_overlay_rect),
    );
    host.register_overlay_selectable(&scope);
    host.register_overlay_slot(scope.clone(), jinn_skills_msg::skill_picker_slot());
    host.register_overlay_view(scope, std::sync::Arc::new(render_skill_picker));

    // The picker's keys, and the filter's input hook, are the slice's own.
    attach_skill_picker_rows(host.key_routes(), &cell);
    register_skill_picker_input_hook(host.key_routes(), &cell);

    // Repaint the picker when a discovery scan reports new skills. Spawned here
    // because this is the slice that owns the picker, and before any scan is
    // published so no result can slip past the subscription.
    //
    // Built with the explicit builder rather than `host.spawn_service`, which
    // omits `.handles(...)`: the actor would spawn and stay subscribed to
    // nothing, so a rescan would update the session but never the open menu.
    trouper::builder::spawn_service_builder::<
        skill_picker_republisher_actor::SkillPickerRepublisherActor,
    >(host.system())
    .at(trouper::actor::ActorPath::new(
        skill_picker_republisher_actor::SkillPickerRepublisherActor::PATH.to_owned(),
    ))
    .start_with(move || {
        Box::pin(async move {
            Ok(skill_picker_republisher_actor::SkillPickerRepublisherActor::new(cell.clone()))
        })
    })
    .handles::<jinn_skills_msg::SkillsLoaded>()
    .start();
}

#[cfg(test)]
mod skill_picker_behavior_tests;

#[cfg(test)]
mod activation_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        reason = "test module, panics are acceptable"
    )]
    use jinn_slices::SliceHost;

    /// The picker is slice-owned, so activation must resolve the cell the
    /// catalog registered. Without this the picker's scope would resolve no
    /// state and render nothing.
    #[rstest::rstest]
    #[tokio::test]
    async fn activate_wires_the_catalog_registered_skill_picker_cell() {
        // Given a host over a registry the catalog has seeded.
        let slices = jinn_slices::Slices::new();
        jinn_cell_catalog::register_all_cells(&slices);
        let mut viewport = jinn_slices::view::Viewport::new();
        let overlay_views = jinn_slices::OverlayViews::new();
        let key_routes = jinn_slices::KeyRoutes::new();
        let services = jinn_kernel::Services::new_fake().await;
        let mut host = SliceHost::new(
            &slices,
            &mut viewport,
            &overlay_views,
            &key_routes,
            &services.trouper_system,
        );

        // When activating the skills slice.
        crate::activate(&mut host);

        // Then the skill picker's cell resolves and is readable.
        let cell = slices
            .reader(&jinn_skills_msg::skill_picker_slot())
            .expect("the catalog registers the skill picker cell activation wires");
        // And it holds the picker's own state type.
        let guard = cell.read();
        let _state: &jinn_skills_msg::SkillPickerState = &guard;
    }
}
