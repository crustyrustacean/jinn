//! Skill picker reload logic.
//!
//! Rebuilds the skill picker entries from the active session's `discovered_skills`,
//! preserving the session's disabled-skills set and the picker's filter text.
use crate::common::app_state::AppState;
use crate::feat::skills::skill_entry::SkillEntry;
use crate::feat::ui::picker_states::PickerExt;

/// Reloads skill picker entries from the active session's discovered skills.
///
/// Reads the active session's `discovered_skills` (cwd-scoped, hydrated by the
/// skills scan actor), so two sessions with different cwds show the right set.
/// Each entry carries the discovered skill's `source` for a global/project badge.
pub fn reload_skill_picker_entries(state: &mut AppState) {
    let disabled = state.active_session().disabled_skills();
    let theme = state.frontend.theme.clone();
    let discovered = state.active_session().discovered_skills().to_vec();

    let mut entries: Vec<SkillEntry> = discovered
        .iter()
        .map(|skill| {
            let name = skill.name.clone();
            let description = skill.description.clone();
            SkillEntry {
                search_text: format!("{name} {description}"),
                name,
                description,
                body: skill.body.clone(),
                enabled: !disabled.contains(&skill.name),
                source: skill.source.clone(),
                theme: theme.clone(),
            }
        })
        .collect();

    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    state.frontend.skill_picker_mut().set_items(entries);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use crate::common::app_state::AppState;
    use crate::common::app_state::FocusScope;
    use crate::feat::skills::skill::{Skill, SkillSource};
    use crate::feat::ui::picker_states::PickerExt;
    use crate::protocol::PickerKind;

    use jinn_selection_widget::PreviewCache;
    use std::path::PathBuf;

    use super::*;

    fn make_skills() -> Vec<Skill> {
        vec![
            Skill {
                name: "web-coder".to_owned(),
                description: "Web dev expert".to_owned(),
                body: "## Web coder body".to_owned(),
                file_path: PathBuf::from("/tmp/skills/web-coder/SKILL.md"),
                base_dir: PathBuf::from("/tmp/skills/web-coder"),
                source: crate::feat::skills::SkillSource::Global,
            },
            Skill {
                name: "phased-task-loop".to_owned(),
                description: "Task loop".to_owned(),
                body: "## Task loop body".to_owned(),
                file_path: PathBuf::from("/tmp/skills/phased-task-loop/SKILL.md"),
                base_dir: PathBuf::from("/tmp/skills/phased-task-loop"),
                source: crate::feat::skills::SkillSource::Global,
            },
        ]
    }
    #[rstest::rstest]
    fn reload_updates_picker_items_from_context_skills() {
        // Given state with skills in the active session's discovered_skills but empty picker.
        let mut state = AppState::default();
        state.active_session_mut().set_discovered_skills(make_skills());
        assert!(state.frontend.skill_picker().items().is_empty());

        // When reloading.
        reload_skill_picker_entries(&mut state);

        // Then the picker has entries matching discovered_skills, sorted alphabetically,
        // each tagged with its source (default Global from make_skills).
        let items = state.frontend.skill_picker().items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "phased-task-loop");
        assert_eq!(items[1].name, "web-coder");
        assert!(items.iter().all(|e| e.source == SkillSource::Global));
    }

    #[rstest::rstest]
    fn reload_marks_disabled_skills_as_not_enabled() {
        // Given state with a disabled skill.
        let mut state = AppState::default();
        state.active_session_mut().set_discovered_skills(make_skills());
        let mut disabled = std::collections::HashSet::new();
        disabled.insert("web-coder".to_owned());
        state.active_session_mut().set_disabled_skills(disabled);

        // When reloading.
        reload_skill_picker_entries(&mut state);

        // Then the disabled skill entry has enabled: false.
        let items = state.frontend.skill_picker().items();
        let web_coder = items
            .iter()
            .find(|e| e.name == "web-coder")
            .expect("should exist");
        assert!(!web_coder.enabled);
        let phased = items
            .iter()
            .find(|e| e.name == "phased-task-loop")
            .expect("should exist");
        assert!(phased.enabled);
    }

    #[rstest::rstest]
    fn reload_preserves_filter_text() {
        // Given state with skills and a filter already set on the picker.
        let mut state = AppState::default();
        state.active_session_mut().set_discovered_skills(make_skills());
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Skill,
        });
        reload_skill_picker_entries(&mut state);
        // Simulate user typing a filter.
        state.frontend.skill_picker_mut().insert_text("web");

        // When reloading.
        reload_skill_picker_entries(&mut state);

        // Then the filter text is preserved.
        assert_eq!(state.frontend.skill_picker().filter(), "web");
    }

    #[rstest::rstest]
    fn reload_preserves_skill_preview_cache() {
        // Given a cache populated with a rendered skill preview.
        let mut state = AppState::default();
        state.active_session_mut().set_discovered_skills(make_skills());
        state.frontend.caches
            .skill_preview_cache
            .write()
            .insert("web-coder".to_owned(), 80, vec![ratatui::text::Line::raw("rendered")]);
        assert_eq!(state.frontend.caches.skill_preview_cache.read().len(), 1);

        // When reloading (e.g. the picker is reopened). The bodies haven't changed,
        // so the cache must survive to avoid re-rendering already-viewed skills.
        reload_skill_picker_entries(&mut state);

        // Then the cache is preserved.
        assert_eq!(state.frontend.caches.skill_preview_cache.read().len(), 1);
    }
}
