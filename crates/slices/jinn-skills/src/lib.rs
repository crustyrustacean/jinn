//! The skills slice — implementation services for agent-skill support.
//!
//! Owns YAML frontmatter parsing, directory scanning, prompt formatting, and
//! the loaded-skill label vocabulary. Portable skill values and crossing
//! contracts live in `jinn-skills-msg`. Everything here is UI-free and
//! kernel-free.

pub mod format;
pub mod frontmatter;
pub mod scan;
pub mod skill;
pub mod skill_picker_actions;
pub mod skill_picker_reload;
pub mod skill_picker_render;
pub mod skill_picker_scope;
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
pub use skill_picker_scope::skill_picker_scope;
pub use skill_preview::render_skill_preview;

/// Activates the slice: mints the skill picker's cell, registers its overlay,
/// and attaches its route rows. No actor.
///
/// # Panics
///
/// Panics if the slot is already registered — double activation is a wiring
/// bug.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate(host: &mut jinn_slices::SliceHost<'_, jinn_slices::RenderFacts>) {
    let _cell = host
        .register_cell(
            jinn_skills_msg::skill_picker_slot(),
            jinn_skills_msg::SkillPickerState::default(),
        )
        .expect("skill picker slot is registered exactly once at wiring");
    let scope = skill_picker_scope();
    host.register_overlay(
        scope.clone(),
        std::sync::Arc::new(skill_picker_overlay_rect),
    );
    host.register_overlay_selectable(&scope);
    host.register_overlay_slot(scope.clone(), jinn_skills_msg::skill_picker_slot());
    host.register_overlay_view(scope, std::sync::Arc::new(render_skill_picker));
}

#[cfg(test)]
mod activation_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        reason = "test module, panics are acceptable"
    )]
    use jinn_slices::SliceHost;

    /// The picker is slice-owned, so activation must mint its cell. Without
    /// this the picker's scope would resolve no state and render nothing.
    #[rstest::rstest]
    #[tokio::test]
    async fn activate_registers_the_skill_picker_cell() {
        // Given a host over an empty slice registry.
        let slices = jinn_slices::Slices::new();
        let mut viewport = jinn_slices::view::Viewport::new();
        let overlay_views = jinn_slices::OverlayViews::new();
        let key_routes = jinn_slices::KeyRoutes::new();
        let services = jinn_domain::Services::new_fake().await;
        let mut host = SliceHost::new(
            &slices,
            &mut viewport,
            &overlay_views,
            &key_routes,
            &services.trouper_system,
        );

        // When activating the skills slice.
        crate::activate(&mut host);

        // Then the skill picker's cell is registered and readable.
        let cell = slices
            .reader(&jinn_skills_msg::skill_picker_slot())
            .expect("skill picker cell registered at activation");
        // And it holds the picker's own state type.
        let guard = cell.read();
        let _state: &jinn_skills_msg::SkillPickerState = &guard;
    }
}
