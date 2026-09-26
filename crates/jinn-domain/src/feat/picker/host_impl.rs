//! The kernel's [`PickerHost`] implementation — the picker crate's lens onto
//! `AppState`.
//!
//! Typed selection storage is lent as `dyn Any` (specs downcast to the
//! exact `SelectionState<PickerEntry<T>>` they own); anything not on the
//! trait flows through `state_any`. This is the *only* place that maps
//! picker ids onto the kernel's typed picker fields.

use jinn_picker::Palette;
use jinn_picker::PickerHost;
use jinn_picker::PickerId;

use crate::common::app_state::AppState;
use crate::feat::ui::picker_states::PickerExt;
use crate::protocol::PickerKind;
use jinn_picker::ENDPOINT_ID;
use jinn_picker::MCP_SERVER_ID;
use jinn_picker::PROJECT_ID;
use jinn_picker::PROVIDER_ID;
use jinn_picker::REASONING_EFFORT_ID;
use jinn_picker::SESSION_ID;
use jinn_picker::SESSION_LIFECYCLE_ID;
use jinn_picker::TASK_LIST_ID;
use jinn_picker::THEME_ID;
use jinn_picker::TOOL_ID;

/// The mutable navigation interface for the active picker, or `None` when no
/// picker is on the focus stack.
///
/// This is the single kind→field table for navigation. Both the host lenses
/// ([`AppStatePickerHost::active_ops`] and
/// [`AppStateRenderHost::active_ops_ref`]) and the kernel-side intent
/// handlers delegate here, so the eleven arms cannot drift apart.
#[must_use]
pub fn active_picker_ops(
    state: &mut AppState,
) -> Option<&mut dyn jinn_selection_widget::PickerOps> {
    let kind = state.frontend.picker_kind()?;
    Some(match kind {
        PickerKind::Provider => &mut state.frontend.pickers.provider_picker,
        PickerKind::Session => state.frontend.session_picker_mut(),
        PickerKind::Theme => state.frontend.theme_picker_mut(),
        PickerKind::SessionLifecycle => state.frontend.session_lifecycle_picker_mut(),
        PickerKind::ReasoningEffort => state.frontend.reasoning_effort_picker_mut(),
        PickerKind::Tool => state.frontend.tool_picker_mut(),
        PickerKind::TaskList => state.frontend.task_list_picker_mut(),
        PickerKind::Project => state.frontend.project_picker_mut(),
        PickerKind::McpServer => state.frontend.mcp_server_picker_mut(),
        PickerKind::Endpoint => state.frontend.endpoint_picker_mut(),
        // Retired: no picker state, never pushed as a scope.
        PickerKind::CompactionModel => return None,
    })
}

/// Read-only companion to [`active_picker_ops`], for the filter-emptiness
/// check behind the universal `CtrlClear` intent.
#[must_use]
pub fn active_picker_ops_ref(state: &AppState) -> Option<&dyn jinn_selection_widget::PickerOps> {
    let kind = state.frontend.picker_kind()?;
    Some(match kind {
        PickerKind::Provider => &state.frontend.pickers.provider_picker,
        PickerKind::Session => state.frontend.session_picker(),
        PickerKind::Theme => state.frontend.theme_picker(),
        PickerKind::SessionLifecycle => state.frontend.session_lifecycle_picker(),
        PickerKind::ReasoningEffort => state.frontend.reasoning_effort_picker(),
        PickerKind::Tool => state.frontend.tool_picker(),
        PickerKind::TaskList => state.frontend.task_list_picker(),
        PickerKind::Project => state.frontend.project_picker(),
        PickerKind::McpServer => state.frontend.mcp_server_picker(),
        PickerKind::Endpoint => state.frontend.endpoint_picker(),
        // Retired: no picker state, never pushed as a scope.
        PickerKind::CompactionModel => return None,
    })
}

/// The read-only selection storage for `id`, or `None` when no picker claims
/// that id.
///
/// The one id→field table for read lends. Both host lenses delegate here, so
/// the mutable and read-only mappings cannot drift: the earlier split let the
/// mutable lens answer `None` for `provider`, `endpoint`, and `project`
/// while the render lens answered correctly, which is a silent blank-popup
/// defect rather than a compile error.
#[must_use]
pub fn selection_state_ref(state: &AppState, id: PickerId) -> Option<&dyn std::any::Any> {
    match id.as_str() {
        THEME_ID => Some(state.frontend.theme_picker() as &dyn std::any::Any),
        TOOL_ID => Some(state.frontend.tool_picker() as &dyn std::any::Any),
        MCP_SERVER_ID => Some(state.frontend.mcp_server_picker() as &dyn std::any::Any),
        SESSION_LIFECYCLE_ID => {
            Some(state.frontend.session_lifecycle_picker() as &dyn std::any::Any)
        }
        REASONING_EFFORT_ID => Some(state.frontend.reasoning_effort_picker() as &dyn std::any::Any),
        TASK_LIST_ID => Some(state.frontend.task_list_picker() as &dyn std::any::Any),
        SESSION_ID => Some(state.frontend.session_picker() as &dyn std::any::Any),
        PROVIDER_ID => Some(&state.frontend.pickers.provider_picker as &dyn std::any::Any),
        ENDPOINT_ID => Some(state.frontend.endpoint_picker() as &dyn std::any::Any),
        PROJECT_ID => Some(state.frontend.project_picker() as &dyn std::any::Any),
        _ => None,
    }
}

/// The host lens over the kernel state. Constructed transiently at
/// dispatch/render with `&mut AppState` — it never outlives the guard.
pub struct AppStatePickerHost<'a> {
    state: &'a mut AppState,
}

impl<'a> AppStatePickerHost<'a> {
    /// Wraps the kernel state.
    #[must_use]
    pub fn new(state: &'a mut AppState) -> Self {
        Self { state }
    }
}

impl PickerHost for AppStatePickerHost<'_> {
    fn selection_state(&mut self, id: PickerId) -> Option<&mut dyn std::any::Any> {
        match id.as_str() {
            THEME_ID => Some(self.state.frontend.theme_picker_mut() as &mut dyn std::any::Any),
            TOOL_ID => Some(self.state.frontend.tool_picker_mut() as &mut dyn std::any::Any),
            MCP_SERVER_ID => {
                Some(self.state.frontend.mcp_server_picker_mut() as &mut dyn std::any::Any)
            }
            SESSION_LIFECYCLE_ID => {
                Some(self.state.frontend.session_lifecycle_picker_mut() as &mut dyn std::any::Any)
            }
            REASONING_EFFORT_ID => {
                Some(self.state.frontend.reasoning_effort_picker_mut() as &mut dyn std::any::Any)
            }
            TASK_LIST_ID => {
                Some(self.state.frontend.task_list_picker_mut() as &mut dyn std::any::Any)
            }
            SESSION_ID => Some(self.state.frontend.session_picker_mut() as &mut dyn std::any::Any),
            PROVIDER_ID => {
                Some(&mut self.state.frontend.pickers.provider_picker as &mut dyn std::any::Any)
            }
            ENDPOINT_ID => {
                Some(self.state.frontend.endpoint_picker_mut() as &mut dyn std::any::Any)
            }
            PROJECT_ID => Some(self.state.frontend.project_picker_mut() as &mut dyn std::any::Any),
            _ => None,
        }
    }

    fn selection_state_ref(&self, id: PickerId) -> Option<&dyn std::any::Any> {
        selection_state_ref(self.state, id)
    }

    fn state_any(&mut self) -> &mut dyn std::any::Any {
        self.state as &mut dyn std::any::Any
    }

    fn state_any_ref(&self) -> &dyn std::any::Any {
        self.state as &dyn std::any::Any
    }

    fn palette(&self) -> Palette {
        let theme = &self.state.frontend.theme;
        // Chrome fields mirror the selection widget's defaults: the pickers
        // do not theme borders/filter/separator, and this palette keeps
        // that look.
        Palette {
            border: ratatui::style::Color::DarkGray,
            filter_text: ratatui::style::Color::White,
            separator: ratatui::style::Color::DarkGray,
            footer: ratatui::style::Color::DarkGray,
            highlight_bg: ratatui::style::Color::DarkGray,
            muted_text: theme.muted_text,
            accent_action: theme.accent_action,
            popup_title: theme.popup_title,
            primary_text: theme.primary_text,
        }
    }

    fn session_id(&self) -> jinn_core_types::SessionId {
        self.state.session.active_session_id().clone()
    }

    fn preview_scroll(&self, id: PickerId) -> usize {
        self.state.frontend.pickers.pickers_scrolls.get(id)
    }

    fn set_preview_scroll(&mut self, id: PickerId, scroll: usize) {
        self.state.frontend.pickers.pickers_scrolls.set(id, scroll);
    }

    fn reset_preview_scroll(&mut self, id: PickerId) {
        self.state.frontend.pickers.pickers_scrolls.reset(id);
    }

    fn preview_cache(&self, _id: PickerId) -> Option<jinn_picker::SharedPreviewCache> {
        // No kernel-side picker owns a preview cache; caches travel with
        // their slice.
        None
    }

    fn active_ops(&mut self) -> Option<&mut dyn jinn_selection_widget::PickerOps> {
        active_picker_ops(self.state)
    }

    fn active_ops_ref(&self) -> Option<&dyn jinn_selection_widget::PickerOps> {
        active_picker_ops_ref(self.state)
    }
}

/// Read-only lens over [`AppState`] for the render path, where no mutable
/// access exists (the render pass holds only a read guard). Read-side host
/// operations are answered; mutable lends are not.
pub struct AppStateRenderHost<'a> {
    state: &'a AppState,
}

impl<'a> AppStateRenderHost<'a> {
    /// Wraps the render pass's state snapshot.
    #[must_use]
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }
}

impl PickerHost for AppStateRenderHost<'_> {
    fn selection_state(&mut self, _id: PickerId) -> Option<&mut dyn std::any::Any> {
        None // render never mutates through this lens
    }

    fn selection_state_ref(&self, id: PickerId) -> Option<&dyn std::any::Any> {
        selection_state_ref(self.state, id)
    }

    #[expect(
        clippy::unreachable,
        reason = "trait contract: render host is read-only; render specs must not mutate state"
    )]
    fn state_any(&mut self) -> &mut dyn std::any::Any {
        unreachable!("AppStateRenderHost is read-only; specs must not call state_any in render")
    }

    fn state_any_ref(&self) -> &dyn std::any::Any {
        self.state
    }

    fn palette(&self) -> Palette {
        let theme = &self.state.frontend.theme;
        // Chrome fields mirror the selection widget's defaults, matching
        // [`AppStatePickerHost::palette`].
        Palette {
            border: ratatui::style::Color::DarkGray,
            filter_text: ratatui::style::Color::White,
            separator: ratatui::style::Color::DarkGray,
            footer: ratatui::style::Color::DarkGray,
            highlight_bg: ratatui::style::Color::DarkGray,
            muted_text: theme.muted_text,
            accent_action: theme.accent_action,
            popup_title: theme.popup_title,
            primary_text: theme.primary_text,
        }
    }

    fn session_id(&self) -> jinn_core_types::SessionId {
        self.state.session.active_session_id().clone()
    }

    fn preview_scroll(&self, id: PickerId) -> usize {
        self.state.frontend.pickers.pickers_scrolls.get(id)
    }

    fn set_preview_scroll(&mut self, _id: PickerId, _scroll: usize) {
        // Read-only lens: render never stores scrolls.
    }

    fn reset_preview_scroll(&mut self, _id: PickerId) {
        // Read-only lens: render never clears scrolls.
    }

    fn preview_cache(&self, _id: PickerId) -> Option<jinn_picker::SharedPreviewCache> {
        // No kernel-side picker owns a preview cache; caches travel with
        // their slice.
        None
    }

    #[expect(
        clippy::unreachable,
        reason = "trait contract: render host is read-only; nothing navigates during a frame"
    )]
    fn active_ops(&mut self) -> Option<&mut dyn jinn_selection_widget::PickerOps> {
        unreachable!("AppStateRenderHost is read-only; render never navigates the active picker")
    }

    fn active_ops_ref(&self) -> Option<&dyn jinn_selection_widget::PickerOps> {
        let kind = self.state.frontend.picker_kind()?;
        Some(match kind {
            PickerKind::Provider => &self.state.frontend.pickers.provider_picker,
            PickerKind::Session => self.state.frontend.session_picker(),
            PickerKind::Theme => self.state.frontend.theme_picker(),
            PickerKind::SessionLifecycle => self.state.frontend.session_lifecycle_picker(),
            PickerKind::ReasoningEffort => self.state.frontend.reasoning_effort_picker(),
            PickerKind::Tool => self.state.frontend.tool_picker(),
            PickerKind::TaskList => self.state.frontend.task_list_picker(),
            PickerKind::Project => self.state.frontend.project_picker(),
            PickerKind::McpServer => self.state.frontend.mcp_server_picker(),
            PickerKind::Endpoint => self.state.frontend.endpoint_picker(),
            // Retired: no picker state, never pushed as a scope.
            PickerKind::CompactionModel => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "test module, panics are acceptable"
    )]
    use super::*;
    use jinn_theme::ThemeEntry;

    fn test_theme(name: &str) -> ThemeEntry {
        ThemeEntry {
            name: name.to_owned(),
            theme: jinn_theme::default_theme(),
        }
    }

    #[rstest::rstest]
    #[test]
    fn selection_state_lends_typed_storage_by_id() {
        // Given a host state whose persona picker holds items.
        let mut state = AppState::default_with_scope_focus();
        let items = jinn_picker::make_items_with_hooks(
            vec![test_theme("a")],
            jinn_picker::PickerItemHooks::new()
                .row(|entry: &ThemeEntry, _ctx: &jinn_picker::RowCtx<'_>| {
                    ratatui::text::Line::raw(entry.name.clone())
                })
                .search(|entry: &ThemeEntry| entry.name.clone()),
        );
        state.frontend.theme_picker_mut().set_items(items);

        // When lending the selection state for the persona id.
        let mapped = {
            let mut host = AppStatePickerHost::new(&mut state);
            host.selection_state(PickerId::new(THEME_ID))
                .expect("theme is mapped")
                .downcast_ref::<jinn_selection_widget::SelectionState<
                    jinn_picker::PickerEntry<ThemeEntry>,
                >>()
                .is_some()
        };

        // Then the lend downcasts back to the wrapped selection storage.
        assert!(
            mapped,
            "persona lend should downcast to its wrapped SelectionState"
        );
    }

    /// Drift guard: every registered spec id must resolve in *both* lenses.
    ///
    /// The two lenses answer the same id→field question, and a gap in either
    /// one is silent: a read lend that returns `None` makes its spec render
    /// nothing at all, with no error. A single shared table plus this guard
    /// keeps the mutable and read-only mappings from drifting apart again.
    #[rstest::rstest]
    #[test]
    fn every_registered_spec_resolves_in_both_host_lenses() {
        // Given a fresh state and the full set of registered picker ids.
        // The id list is spelled out rather than pulled from the registry:
        // `jinn-domain` cannot depend on `jinn-picker-specs` (that dependency
        // is the cycle this whole migration exists to remove), and listing
        // them keeps this guard a real compile-time-complete check of the
        // table above.
        let ids = [
            jinn_picker::THEME_ID,
            jinn_picker::TOOL_ID,
            jinn_picker::MCP_SERVER_ID,
            jinn_picker::SESSION_LIFECYCLE_ID,
            jinn_picker::REASONING_EFFORT_ID,
            jinn_picker::TASK_LIST_ID,
            jinn_picker::SESSION_ID,
            jinn_picker::PROVIDER_ID,
            jinn_picker::ENDPOINT_ID,
            jinn_picker::PROJECT_ID,
        ];
        let mut state = AppState::default_with_scope_focus();

        // When lending each id through the read lens and the mutable lens.
        let mut unresolved = Vec::new();
        for id in &ids {
            let picker_id = PickerId::new(id);
            if selection_state_ref(&state, picker_id).is_none() {
                unresolved.push(format!("{id} (read)"));
            }
            if AppStatePickerHost::new(&mut state)
                .selection_state_ref(picker_id)
                .is_none()
            {
                unresolved.push(format!("{id} (read-lens-via-host)"));
            }
            if AppStatePickerHost::new(&mut state)
                .selection_state(picker_id)
                .is_none()
            {
                unresolved.push(format!("{id} (mut)"));
            }
            if AppStateRenderHost::new(&state)
                .selection_state_ref(picker_id)
                .is_none()
            {
                unresolved.push(format!("{id} (render)"));
            }
        }

        // Then every id resolves everywhere.
        assert!(
            unresolved.is_empty(),
            "picker ids that lend no storage in some lens (silent blank popup): {unresolved:?}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn unmapped_ids_lend_nothing() {
        // Given a default host state.
        let mut state = AppState::default_with_scope_focus();
        let mut host = AppStatePickerHost::new(&mut state);

        // When lending an id no picker claims.
        // Then nothing is returned.
        assert!(host.selection_state(PickerId::new("nope")).is_none());
    }

    #[rstest::rstest]
    #[test]
    fn preview_scrolls_are_stored_per_picker_id() {
        // Given a host state.
        let mut state = AppState::default_with_scope_focus();
        let first = PickerId::new(THEME_ID);
        let other = PickerId::new("other");

        // When setting preview scrolls for one picker id and another id.
        let (first_scroll, other_scroll, stored) = {
            let mut host = AppStatePickerHost::new(&mut state);
            host.set_preview_scroll(first, 7);
            host.set_preview_scroll(other, 3);
            (
                PickerHost::preview_scroll(&host, first),
                PickerHost::preview_scroll(&host, other),
                state.frontend.pickers.pickers_scrolls.get(first),
            )
        };

        // Then both scrolls live in the shared map, keyed by picker id.
        assert_eq!(first_scroll, 7);
        assert_eq!(stored, 7);
        assert_eq!(other_scroll, 3);
    }
}
