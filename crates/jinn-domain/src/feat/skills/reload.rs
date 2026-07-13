//! Skill picker reload logic.
//!
//! Rebuilds the skill picker entries from the active session's `discovered_skills`,
//! preserving the session's disabled-skills set and the picker's filter text.
use crate::feat::skills::Skill;
use crate::feat::skills::skill_entry::SkillEntry;
use crate::feat::ui::frontend_state::FrontendState;
use crate::feat::ui::picker_states::PickerExt;
use std::collections::HashSet;

/// Reloads skill picker entries from the active session's discovered skills.
///
/// Reads the active session's `discovered_skills` (cwd-scoped, hydrated by the
/// skills scan actor), so two sessions with different cwds show the right set.
/// Each entry carries the discovered skill's `source` for a global/project badge.
///
/// The session data (`discovered_skills`, `disabled_skills`) is read from a
/// snapshot and passed in, so this function only writes `frontend` — it does
/// not need `&mut AppState`.
pub fn reload_skill_picker_entries(
    frontend: &mut FrontendState,
    discovered: &[Skill],
    disabled: &HashSet<String>,
    theme: &crate::feat::theme::Theme,
) {
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

    entries.sort_by_key(|e| e.name.to_lowercase());

    frontend.skill_picker_mut().set_items(entries);
}
