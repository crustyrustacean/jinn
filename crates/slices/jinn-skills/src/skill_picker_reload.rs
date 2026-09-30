//! Skill picker reload logic.
//!
//! Rebuilds the skill picker entries from the session's `discovered_skills`,
//! preserving the disabled-skills set and the picker's filter text. Owned by
//! the skills slice: the picker writes into this slice's own
//! [`SkillPickerState`] cell rather than a kernel-owned field, which is what
//! lets the picker leave the kernel's vocabulary behind.

use crate::skill_preview::render_skill_preview;
use jinn_core_types::NameFilter;
use jinn_skills_msg::{Skill, SkillEntry, SkillPickerState, body_hash_key, skill_row};
use jinn_theme::Theme;

/// Wraps `discovered` into the picker's rows, marking each enabled unless its
/// name is in `disabled`, sorted case-insensitively by name.
///
/// Entries are wrapped through the same hooks the skill spec declares, so the
/// reload path and the spec path cannot drift in how a row or preview draws.
#[must_use]
pub fn build_skill_entries(
    discovered: &[Skill],
    skill_filter: Option<&NameFilter>,
    theme: &Theme,
) -> Vec<jinn_picker::PickerEntry<SkillEntry>> {
    let mut entries: Vec<SkillEntry> = discovered
        .iter()
        .map(|skill| SkillEntry {
            name: skill.name.clone(),
            description: skill.description.clone(),
            body: skill.body.clone(),
            // Seeded from the filter itself rather than from a withheld
            // list, so a skill the session's allow-mode filter omits opens as
            // already off rather than silently toggled on.
            enabled: skill_filter.is_none_or(|filter| filter.permits(&skill.name)),
            source: skill.source.clone(),
            theme: theme.clone(),
        })
        .collect();

    entries.sort_by_key(|e| e.name.to_lowercase());

    jinn_picker::make_items_with_hooks(
        entries,
        jinn_picker::PickerItemHooks::new()
            .row(skill_row)
            .search(|entry: &SkillEntry| format!("{} {}", entry.name, entry.description))
            .preview(render_skill_preview)
            .preview_key(|entry: &SkillEntry| {
                Some(jinn_picker::PreviewKey(body_hash_key(&entry.body)))
            }),
    )
}

/// Reloads the picker's rows from the session's discovered skills.
///
/// `state` is the skill picker's own cell. The session data
/// (`discovered_skills`, the skill filter) is passed in rather than read from
/// the session, so this stays a pure function of (skills, filter, theme).
pub fn reload_skill_picker(
    state: &mut SkillPickerState,
    discovered: &[Skill],
    skill_filter: Option<&NameFilter>,
    theme: &Theme,
) {
    let wrapped = build_skill_entries(discovered, skill_filter, theme);
    state.selection.set_items(wrapped);
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
        // Given a skill picker cell and one discovered skill.
        let mut state = SkillPickerState::default();
        let skills = vec![skill("alpha", "does things", "# body")];

        // When reloading the skill picker.
        reload_skill_picker(&mut state, &skills, None, &default_theme());

        // Then the row renders through the spec's row hook (enabled marker
        // plus name), not the bare search label.
        let row = state.selection.items()[0].render_row(false);
        let text: String = row.spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(text, "✓ alpha");
    }

    #[rstest::rstest]
    fn reload_keeps_the_spec_preview_renderer() {
        // Given a skill picker cell and one discovered skill with a markdown body.
        let mut state = SkillPickerState::default();
        let skills = vec![skill("alpha", "does things", "# Title")];

        // When reloading the skill picker.
        reload_skill_picker(&mut state, &skills, None, &default_theme());

        // Then the preview renders the markdown body, not an empty pane.
        let item = &state.selection.items()[0];
        let lines = item.preview_lines(80);
        assert!(
            lines.iter().any(|l| l.to_string().contains("Title")),
            "preview should render the body; got {lines:?}"
        );
    }

    #[rstest::rstest]
    fn reload_sorts_entries_case_insensitively_by_name() {
        // Given discovered skills in non-alphabetical order.
        let skills = vec![
            skill("zeta", "last", ""),
            skill("Alpha", "first", ""),
            skill("mid", "middle", ""),
        ];

        // When building the picker's entries.
        let items = build_skill_entries(&skills, None, &default_theme());

        // Then they are ordered case-insensitively. The label is the spec's
        // search text, "{name} {description}", so the leading token is the name.
        let names: Vec<&str> = items
            .iter()
            .map(jinn_selection_widget::TreeItem::display_label)
            .map(|label| label.split(' ').next().unwrap_or(label))
            .collect();
        assert_eq!(names, ["Alpha", "mid", "zeta"]);
    }
}
