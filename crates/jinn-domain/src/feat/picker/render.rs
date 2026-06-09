//! Picker rendering for persona and theme pickers.

use crate::common::render_ctx::RenderCtx;
use crate::feat::ui::picker_states::PickerExt;
use jinn_selection_widget::PreviewSelectionWidget;
use jinn_selection_widget::SelectionWidget;
use jinn_selection_widget::TreePickerWidget;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;

/// Renders the persona picker overlay using [`SelectionWidget`].
///
/// Telescope-style layout: bordered popup with filter input at top,
/// horizontal separator, scrollable persona entries.
pub fn render_persona_picker(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    let state = ctx.state;
    let footer = {
        use ratatui::style::Style;
        use ratatui::text::{Line, Span};
        let gray = Style::default().fg(state.frontend.theme.muted_text);
        let active_name = state
            .context
            .active_persona
            .as_ref()
            .map_or("none", |p| p.name.as_str());
        Line::from(vec![
            Span::styled("Active: ".to_owned(), gray),
            Span::styled(
                active_name.to_owned(),
                Style::default().fg(state.frontend.theme.primary_text),
            ),
        ])
    };
    let widget = SelectionWidget::new(state.frontend.persona_picker())
        .title(Line::from(" Personas "))
        .title_style(Style::default().fg(state.frontend.theme.popup_title))
        .footer(footer);
    widget.render(frame, area);
}

/// Renders the theme picker overlay using [`SelectionWidget`].
///
/// Telescope-style layout: bordered popup with filter input at top,
/// horizontal separator, scrollable theme entries.
pub fn render_theme_picker(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    let state = ctx.state;
    let widget = SelectionWidget::new(state.frontend.theme_picker())
        .title(Line::from(" Themes "))
        .title_style(Style::default().fg(state.frontend.theme.popup_title))
        .footer(Line::from(" ESC to cancel, Enter to apply "));
    widget.render(frame, area);
}

/// Renders the plugin picker overlay using [`SelectionWidget`].
///
/// Telescope-style layout: bordered popup with filter input at top,
/// horizontal separator, scrollable plugin entries.
pub fn render_plugin_picker(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    let state = ctx.state;
    let widget = SelectionWidget::new(state.frontend.plugin_picker())
        .title(Line::from(" Plugins "))
        .title_style(Style::default().fg(state.frontend.theme.popup_title))
        .footer(Line::from(" Enter to run, ESC to cancel "));
    widget.render(frame, area);
}

/// Renders the tool picker overlay.
pub fn render_tool_picker(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    let state = ctx.state;
    let enabled_count = state
        .frontend
        .tool_picker()
        .items()
        .iter()
        .filter(|t| t.enabled)
        .count();
    let total = state.frontend.tool_picker().items().len();
    let footer = Line::from(format!(
        " TAB toggle \u{00b7} {enabled_count}/{total} enabled \u{00b7} Enter confirm \u{00b7} ESC cancel "
    ));
    let widget = SelectionWidget::new(state.frontend.tool_picker())
        .title(Line::from(" Tools "))
        .title_style(Style::default().fg(state.frontend.theme.popup_title))
        .footer(footer);
    widget.render(frame, area);
}

/// Renders the skill picker overlay with a preview pane.
///
/// Uses [`PreviewSelectionWidget`] to show the selected skill's markdown body
/// in a split pane (vertical on wide terminals, horizontal on narrow ones).
pub fn render_skill_picker(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    let state = ctx.state;
    let enabled_count = state
        .frontend
        .skill_picker()
        .items()
        .iter()
        .filter(|s| s.enabled)
        .count();
    let total = state.frontend.skill_picker().items().len();
    let gray = Style::default().fg(state.frontend.theme.muted_text);
    let orange = Style::default().fg(state.frontend.theme.accent_action);
    let footer = Line::from(vec![
        ratatui::text::Span::styled("CTRL+R to refresh ".to_owned(), orange),
        ratatui::text::Span::styled(
            format!(
                "\u{00b7} {enabled_count}/{total} enabled \u{00b7} Enter confirm \u{00b7} ESC cancel"
            ),
            gray,
        ),
    ]);
    let cache = state.frontend.caches.skill_preview_cache.write();
    let widget = PreviewSelectionWidget::new(state.frontend.skill_picker())
        .title(Line::from(" Skills "))
        .title_style(Style::default().fg(state.frontend.theme.popup_title))
        .preview_scroll(state.frontend.skill_preview_scroll())
        .footer(footer)
        .preview_cache(&*cache);
    widget.render(frame, area);
}

/// Renders the read-only task list browser overlay.
///
/// Uses [`TreePickerWidget`] to show phases as roots and tasks as their children,
/// fully expanded. Enter is a no-op; ESC/Q close the picker.
pub fn render_task_list_picker(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    let state = ctx.state;
    let gray = Style::default().fg(state.frontend.theme.muted_text);
    let footer = Line::from(vec![ratatui::text::Span::styled(
        "ESC to close \u{00b7} Enter no-op \u{00b7} type to filter".to_owned(),
        gray,
    )]);
    let widget = TreePickerWidget::new(state.frontend.task_list_picker())
        .title(Line::from(" Task List "))
        .title_style(Style::default().fg(state.frontend.theme.popup_title))
        .footer(footer);
    widget.render(frame, area);
}

#[cfg(test)]
mod tests {
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, clippy::unreachable, clippy::string_slice, clippy::uninlined_format_args, reason = "test code")]
    use super::*;
    use crate::common::app_state::{AppState, FocusScope};
    use crate::common::render_ctx::RenderCtx;
    use crate::feat::skills::reload::reload_skill_picker_entries;
    use crate::feat::ui::picker_states::PickerExt;
    use crate::protocol::PickerKind;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Rendering the skill picker with the same selection and width populates the
    /// preview cache exactly once; the second render is a cache hit (no re-render).
    #[test]
    fn render_skill_picker_caches_preview_per_skill_and_width() {
        // Given a picker populated with two skills and a selection on the first.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .set_discovered_skills(vec![crate::feat::skills::Skill {
                name: "web-coder".to_owned(),
                description: "Web coder".to_owned(),
                body: "## Body text that renders".to_owned(),
                file_path: std::path::PathBuf::from("/tmp/web-coder/SKILL.md"),
                base_dir: std::path::PathBuf::from("/tmp/web-coder"),
                source: crate::feat::skills::SkillSource::Global,
            }]);
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Skill,
        });
        reload_skill_picker_entries(&mut state);
        state.frontend.skill_picker_mut().set_selection(0);
        assert!(state.frontend.caches.skill_preview_cache.read().is_empty());

        // When the picker is rendered twice at the same width.
        let area = Rect::new(0, 0, 100, 30);
        for _ in 0..2 {
            let backend = TestBackend::new(100, 30);
            let mut terminal = Terminal::new(backend).expect("terminal");
            terminal
                .draw(|frame| {
                    let ctx = RenderCtx::new(&state);
                    render_skill_picker(frame, area, &ctx);
                })
                .expect("draw");
        }

        // Then the cache holds exactly one entry; the second render was a cache hit.
        let cache = state.frontend.caches.skill_preview_cache.read();
        assert_eq!(
            cache.len(),
            1,
            "second render should be a cache hit, not a second insert"
        );
        // The entry is for the selected skill regardless of the popup's computed width.
        assert_eq!(
            cache.skill_names(),
            vec!["web-coder".to_owned()],
            "only the selected skill's preview should be cached"
        );
    }

    /// Navigating from skill A -> B -> A should not re-render A on return; the
    /// cache should hold exactly {A, B} entries, proving A was a hit on the way back.
    #[test]
    fn render_skill_picker_switch_and_back_does_not_re_render_cached_skill() {
        // Given a picker with two skills, selection on the first.
        let mut state = AppState::default();
        state.active_session_mut().set_discovered_skills(vec![
            crate::feat::skills::Skill {
                name: "web-coder".to_owned(),
                description: "Web coder".to_owned(),
                body: "## Web body".to_owned(),
                file_path: std::path::PathBuf::from("/tmp/web-coder/SKILL.md"),
                base_dir: std::path::PathBuf::from("/tmp/web-coder"),
                source: crate::feat::skills::SkillSource::Global,
            },
            crate::feat::skills::Skill {
                name: "rust".to_owned(),
                description: "Rust".to_owned(),
                body: "## Rust body".to_owned(),
                file_path: std::path::PathBuf::from("/tmp/rust/SKILL.md"),
                base_dir: std::path::PathBuf::from("/tmp/rust"),
                source: crate::feat::skills::SkillSource::Global,
            },
        ]);
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Skill,
        });
        reload_skill_picker_entries(&mut state);
        state.frontend.skill_picker_mut().set_selection(0);

        let draw = |state: &AppState, area: Rect| {
            let backend = TestBackend::new(area.width, area.height);
            let mut terminal = Terminal::new(backend).expect("terminal");
            terminal
                .draw(|frame| {
                    let ctx = RenderCtx::new(state);
                    render_skill_picker(frame, area, &ctx);
                })
                .expect("draw");
        };

        let area = Rect::new(0, 0, 100, 30);
        // Render skill A (web-coder) - populates cache with A.
        draw(&state, area);
        assert_eq!(state.frontend.caches.skill_preview_cache.read().len(), 1);

        // When navigating to skill B and rendering.
        state.frontend.skill_picker_mut().set_selection(1);
        draw(&state, area);
        assert_eq!(
            state.frontend.caches.skill_preview_cache.read().len(),
            2,
            "navigating to B should add a second entry"
        );

        // Then navigating back to A should NOT add a third entry (A is a cache hit).
        state.frontend.skill_picker_mut().set_selection(0);
        draw(&state, area);
        let cache = state.frontend.caches.skill_preview_cache.read();
        assert_eq!(
            cache.len(),
            2,
            "returning to A should be a cache hit, not a re-render"
        );
        let mut names = cache.skill_names();
        names.sort();
        assert_eq!(names, vec!["rust".to_owned(), "web-coder".to_owned()]);
    }

    /// Resizing the terminal (width change) re-renders for the new width; the cache
    /// holds two width-keyed entries for the same skill.
    #[test]
    fn render_skill_picker_width_change_creates_new_cache_entry() {
        // Given a picker with one skill.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .set_discovered_skills(vec![crate::feat::skills::Skill {
                name: "web-coder".to_owned(),
                description: "Web coder".to_owned(),
                body: "## Body text".to_owned(),
                file_path: std::path::PathBuf::from("/tmp/web-coder/SKILL.md"),
                base_dir: std::path::PathBuf::from("/tmp/web-coder"),
                source: crate::feat::skills::SkillSource::Global,
            }]);
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: PickerKind::Skill,
        });
        reload_skill_picker_entries(&mut state);
        state.frontend.skill_picker_mut().set_selection(0);

        let draw = |state: &AppState, area: Rect| {
            let backend = TestBackend::new(area.width, area.height);
            let mut terminal = Terminal::new(backend).expect("terminal");
            terminal
                .draw(|frame| {
                    let ctx = RenderCtx::new(state);
                    render_skill_picker(frame, area, &ctx);
                })
                .expect("draw");
        };

        // When rendering at width 100 then width 140.
        // Both produce popups >= VERTICAL_SPLIT_MIN_WIDTH (101) only for 140;
        // the popup width differs (100-term => 80-col popup; 140-term => 112-col popup),
        // so the width keys must differ.
        draw(&state, Rect::new(0, 0, 100, 30));
        let width_100_entry_count = state.frontend.caches.skill_preview_cache.read().len();
        draw(&state, Rect::new(0, 0, 140, 30));
        let cache = state.frontend.caches.skill_preview_cache.read();

        // Then the cache holds two entries: one per width key.
        assert_eq!(
            width_100_entry_count, 1,
            "first render populates exactly one entry"
        );
        assert_eq!(
            cache.len(),
            2,
            "width change should create a second width-keyed entry, not overwrite"
        );
        // Then the cache holds two entries: one per width key. The single skill
        // name appears under both widths (not deduplicated by skill_names).
        let mut names = cache.skill_names();
        names.sort();
        assert_eq!(
            cache.len(),
            2,
            "width change should create a second width-keyed entry, not overwrite"
        );
        assert_eq!(
            names,
            vec!["web-coder".to_owned(), "web-coder".to_owned()],
            "same skill name appears under two width keys"
        );
    }
}
