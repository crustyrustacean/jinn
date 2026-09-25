//! Skill picker reload — the kernel's seam onto the skills slice.
//!
//! The rebuild itself (entry construction, wrapping, ordering) lives in
//! `jinn-skills`, which owns the picker. What remains here is the kernel's
//! access path: the picker is still reached through `FrontendState` until the
//! cell migration lands, so this bridges the two shapes.
use crate::feat::ui::frontend_state::FrontendState;
use crate::feat::ui::picker_states::PickerExt;
use jinn_skills_msg::Skill;
use std::collections::HashSet;

/// Reloads skill picker entries from the session's discovered skills.
///
/// Reads the active session's `discovered_skills` (cwd-scoped, hydrated by the
/// skills scan actor), so two sessions with different cwds show the right set.
/// Each entry carries the discovered skill's `source` for a global/project badge.
///
/// Session data (`discovered_skills`, `disabled_skills`) is passed in, so this
/// only writes `frontend` — it does not need `&mut AppState`.
pub fn reload_skill_picker_entries(
    frontend: &mut FrontendState,
    discovered: &[Skill],
    disabled: &HashSet<String>,
    theme: &jinn_theme::Theme,
) {
    let wrapped = jinn_skills::build_skill_entries(discovered, disabled, theme);
    frontend.skill_picker_mut().set_items(wrapped);
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "test module, panics are acceptable"
    )]
    use super::*;
    use jinn_selection_widget::PreviewContent;
    use jinn_selection_widget::TreeItem;
    use jinn_theme::default_theme;

    fn skill(name: &str, description: &str, body: &str) -> Skill {
        Skill {
            name: name.to_owned(),
            description: description.to_owned(),
            body: body.to_owned(),
            file_path: std::path::PathBuf::from(format!("/tmp/{name}/SKILL.md")),
            base_dir: std::path::PathBuf::from(format!("/tmp/{name}")),
            source: jinn_skills_msg::SkillSource::Global,
        }
    }

    #[rstest::rstest]
    fn reload_keeps_the_spec_row_renderer() {
        // Given a frontend and one discovered skill.
        let mut frontend = FrontendState::default();
        let skills = vec![skill("alpha", "does things", "# body")];

        // When reloading the skill picker.
        reload_skill_picker_entries(&mut frontend, &skills, &HashSet::new(), &default_theme());

        // Then the row renders through the spec's row hook (enabled marker
        // plus name), not the bare search label.
        let row = frontend.skill_picker().items()[0].render_row(false);
        let text: String = row.spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(text, "✓ alpha");
    }

    #[rstest::rstest]
    fn reload_keeps_the_spec_preview_renderer() {
        // Given a frontend and one discovered skill with a markdown body.
        let mut frontend = FrontendState::default();
        let skills = vec![skill("alpha", "does things", "# Title")];

        // When reloading the skill picker.
        reload_skill_picker_entries(&mut frontend, &skills, &HashSet::new(), &default_theme());

        // Then the preview renders the markdown body, not an empty pane.
        let item = &frontend.skill_picker().items()[0];
        let lines = item.preview_lines(80);
        assert!(
            lines.iter().any(|l| l.to_string().contains("Title")),
            "preview should render the body; got {lines:?}"
        );
    }
}
