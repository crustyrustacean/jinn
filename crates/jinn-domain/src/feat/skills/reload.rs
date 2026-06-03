//! Skill picker reload logic.
//!
//! Rebuilds the skill picker entries from the current skills in `AppState.context.skills`,
//! preserving the active session's disabled-skills set and the picker's filter text.

use crate::common::app_state::AppState;
use crate::feat::skills::skill_entry::SkillEntry;
use crate::feat::ui::picker_states::PickerExt;

/// Reloads skill picker entries from the current skills in AppState.
///
/// Reads `state.context.skills` and the active session's `disabled_skills` set.
/// Builds sorted `SkillEntry` items and calls `set_items` on the skill picker,
/// which preserves the current filter text and clamps selection.
pub fn reload_skill_picker_entries(state: &mut AppState) {
    let disabled = state.active_session().disabled_skills();
    let theme = state.frontend.theme.clone();

    let mut entries: Vec<SkillEntry> = state
        .context
        .skills
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
    use crate::feat::skills::skill::Skill;
    use crate::feat::ui::picker_states::PickerExt;
    use crate::protocol::PickerKind;
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
            },
            Skill {
                name: "phased-task-loop".to_owned(),
                description: "Task loop".to_owned(),
                body: "## Task loop body".to_owned(),
                file_path: PathBuf::from("/tmp/skills/phased-task-loop/SKILL.md"),
                base_dir: PathBuf::from("/tmp/skills/phased-task-loop"),
            },
        ]
    }
    #[rstest::rstest]
    fn reload_updates_picker_items_from_context_skills() {
        // Given state with skills in context.skills but empty picker.
        let mut state = AppState::default();
        state.context.skills = make_skills();
        assert!(state.frontend.skill_picker().items().is_empty());

        // When reloading.
        reload_skill_picker_entries(&mut state);

        // Then the picker has entries matching context.skills, sorted alphabetically.
        let items = state.frontend.skill_picker().items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "phased-task-loop");
        assert_eq!(items[1].name, "web-coder");
    }

    #[rstest::rstest]
    fn reload_marks_disabled_skills_as_not_enabled() {
        // Given state with a disabled skill.
        let mut state = AppState::default();
        state.context.skills = make_skills();
        let mut disabled = std::collections::HashSet::new();
        disabled.insert("web-coder".to_owned());
        state
            .active_session_mut()
            .set_disabled_skills(disabled);

        // When reloading.
        reload_skill_picker_entries(&mut state);

        // Then the disabled skill entry has enabled: false.
        let items = state.frontend.skill_picker().items();
        let web_coder = items.iter().find(|e| e.name == "web-coder").expect("should exist");
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
        state.context.skills = make_skills();
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
}
