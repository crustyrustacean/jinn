//! Skill picker reload logic.
//!
//! Rebuilds the skill picker entries from the active session's `discovered_skills`,
//! preserving the session's disabled-skills set and the picker's filter text.
use crate::feat::ui::frontend_state::FrontendState;
use crate::feat::ui::picker_states::PickerExt;
use jinn_skills_msg::Skill;
use std::collections::HashSet;

/// Reloads skill picker entries from the active session's discovered skills.
///
/// Reads the active session's `discovered_skills` (cwd-scoped, hydrated by the
/// skills scan actor), so two sessions with different cwds show the right set.
/// Each entry carries the discovered skill's `source` for a global/project badge.
///
/// The session data (`discovered_skills`, `disabled_skills`) is read from a
/// snapshot and passed in, so this function only writes `frontend` — it does
/// not need `&mut AppState`. Entries are wrapped into `PickerEntry`s through
/// the skill spec's render/search hooks, exactly as the picker renders them.
pub fn reload_skill_picker_entries(
    frontend: &mut FrontendState,
    discovered: &[Skill],
    disabled: &HashSet<String>,
    theme: &jinn_theme::Theme,
) {
    let mut entries: Vec<crate::feat::skills::skill_entry::SkillEntry> = discovered
        .iter()
        .map(|skill| {
            let name = skill.name.clone();
            let description = skill.description.clone();
            crate::feat::skills::skill_entry::SkillEntry {
                name,
                description,
                body: skill.body.clone(),
                enabled: !disabled.contains(&skill.name),
                source: skill.source.clone(),
                theme: theme.clone(),
            }
        })
        .collect();

    entries.sort_by_key(|e| e.name.to_lowercase());

    let wrapped = jinn_picker::make_items_with_hooks(
        entries,
        jinn_picker::PickerItemHooks::new()
            .row(crate::feat::skills::skill_entry::skill_row)
            .search(|entry: &crate::feat::skills::skill_entry::SkillEntry| {
                format!("{} {}", entry.name, entry.description)
            })
            .preview(crate::feat::skills::skill_entry::render_skill_preview)
            .preview_key(|entry: &crate::feat::skills::skill_entry::SkillEntry| {
                Some(jinn_picker::PreviewKey(
                    crate::feat::skills::skill_entry::body_hash_key(&entry.body),
                ))
            }),
    );
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
