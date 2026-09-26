//! The skill picker's preview renderer.
//!
//! Renders a skill's markdown body into the picker's preview pane. This lives
//! in the slice rather than in `jinn-skills-msg` because the markdown
//! renderer lives in `jinn-chat-log-view`, which already depends on
//! `jinn-skills` — putting it in the msg crate would close a cycle.

use jinn_skills_msg::SkillEntry;
use ratatui::text::Line;

/// Renders the skill's markdown body for the preview pane.
pub fn render_skill_preview(
    entry: &SkillEntry,
    ctx: &jinn_picker::PreviewCtx<'_>,
) -> Vec<Line<'static>> {
    if entry.body.is_empty() {
        return Vec::new();
    }
    jinn_chat_log_view::chat_log::render_markdown(&entry.body, ctx.width as u16, &entry.theme)
}
