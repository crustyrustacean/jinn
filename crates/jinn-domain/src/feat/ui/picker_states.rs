//! Picker state grouping and accessor trait.
//!
//! All picker-related state (picker widgets, snapshots, scroll offsets) is grouped
//! into [`PickerStates`]. The [`PickerExt`] extension trait provides accessor methods
//! on [`FrontendState`](super::FrontendState) so consumers are decoupled from the
//! internal storage layout.

use std::collections::HashSet;


use crate::feat::persona::PersonaEntry;
use crate::feat::session::picker_entry::SessionTreeEntry;
use crate::feat::session_lifecycle::picker_entry::SessionLifecycleEntry;
use crate::feat::skills::skill_entry::SkillEntry;
use crate::feat::theme::Theme;
use crate::feat::theme::ThemeEntry;
use crate::feat::tools_actor::tool_entry::ToolEntry;
use crate::feat::workflow::picker_entry::WorkflowPickerEntry;
use crate::protocol::PickerEntry;

/// All picker state - grouped so the picker subsystem can evolve independently.
///
/// Each picker has its own selection state and optional companion fields
/// (snapshots, scroll offsets) used during the picker's open/close lifecycle.
#[derive(Debug, Default)]
pub struct PickerStates {
    /// Session picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (session picker navigation).
    pub session_picker: jinn_selection_widget::TreePickerState<SessionTreeEntry>,

    /// Persona picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (persona picker navigation).
    pub persona_picker: jinn_selection_widget::SelectionState<PersonaEntry>,

    /// Theme picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (theme picker navigation).
    pub theme_picker: jinn_selection_widget::SelectionState<ThemeEntry>,

    /// Saved theme before preview - restored on ESC.
    /// OWNER: IntentHandler (set on theme picker open, consumed on confirm/cancel).
    pub theme_preview_original: Option<Theme>,

    /// Tool picker state - shows all registered tools with toggle state.
    /// OWNER: IntentHandler (populated on tool picker open).
    pub tool_picker: jinn_selection_widget::SelectionState<ToolEntry>,

    /// Snapshot of disabled tools before picker opens - restored on ESC.
    /// OWNER: IntentHandler (set on tool picker open, consumed on confirm/cancel).
    pub tool_picker_snapshot: Option<HashSet<String>>,

    /// Skill picker state - shows all discovered skills with toggle state.
    /// OWNER: IntentHandler (populated on skill picker open).
    pub skill_picker: jinn_selection_widget::SelectionState<SkillEntry>,

    /// Snapshot of disabled skills before picker opens - restored on ESC.
    /// OWNER: IntentHandler (set on skill picker open, consumed on confirm/cancel).
    pub skill_picker_snapshot: Option<HashSet<String>>,

    /// Preview pane scroll offset for the skill picker.
    /// Reset to 0 when the selection changes.
    pub skill_preview_scroll: usize,

    /// Session lifecycle picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (lifecycle picker navigation).
    pub session_lifecycle_picker: jinn_selection_widget::SelectionState<SessionLifecycleEntry>,

    /// Workflow picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (workflow picker navigation) + WorkflowActor (entry population).
    pub workflow_picker: jinn_selection_widget::SelectionState<WorkflowPickerEntry>,


    /// Compaction model picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (compaction model picker navigation).
    pub compaction_model_picker: jinn_selection_widget::SelectionState<PickerEntry>,
}

/// Extension trait providing typed access to picker state on [`FrontendState`](super::FrontendState).
///
/// Import this trait to access picker fields through methods instead of direct field access.
/// This decouples consumers from the internal storage layout of `FrontendState`.
pub trait PickerExt {
    // --- Session picker ---

    /// Read-only access to the session picker state.
    fn session_picker(&self) -> &jinn_selection_widget::TreePickerState<SessionTreeEntry>;
    /// Mutable access to the session picker state.
    fn session_picker_mut(&mut self) -> &mut jinn_selection_widget::TreePickerState<SessionTreeEntry>;

    // --- Persona picker ---

    /// Read-only access to the persona picker state.
    fn persona_picker(&self) -> &jinn_selection_widget::SelectionState<PersonaEntry>;
    /// Mutable access to the persona picker state.
    fn persona_picker_mut(&mut self) -> &mut jinn_selection_widget::SelectionState<PersonaEntry>;

    // --- Theme picker ---

    /// Read-only access to the theme picker state.
    fn theme_picker(&self) -> &jinn_selection_widget::SelectionState<ThemeEntry>;
    /// Mutable access to the theme picker state.
    fn theme_picker_mut(&mut self) -> &mut jinn_selection_widget::SelectionState<ThemeEntry>;
    /// Read-only access to the saved theme before preview.
    fn theme_preview_original(&self) -> &Option<Theme>;
    /// Mutable access to the saved theme before preview.
    fn theme_preview_original_mut(&mut self) -> &mut Option<Theme>;

    // --- Tool picker ---

    /// Read-only access to the tool picker state.
    fn tool_picker(&self) -> &jinn_selection_widget::SelectionState<ToolEntry>;
    /// Mutable access to the tool picker state.
    fn tool_picker_mut(&mut self) -> &mut jinn_selection_widget::SelectionState<ToolEntry>;
    /// Read-only access to the disabled tools snapshot.
    fn tool_picker_snapshot(&self) -> &Option<HashSet<String>>;
    /// Mutable access to the disabled tools snapshot.
    fn tool_picker_snapshot_mut(&mut self) -> &mut Option<HashSet<String>>;

    // --- Skill picker ---

    /// Read-only access to the skill picker state.
    fn skill_picker(&self) -> &jinn_selection_widget::SelectionState<SkillEntry>;
    /// Mutable access to the skill picker state.
    fn skill_picker_mut(&mut self) -> &mut jinn_selection_widget::SelectionState<SkillEntry>;
    /// Read-only access to the disabled skills snapshot.
    fn skill_picker_snapshot(&self) -> &Option<HashSet<String>>;
    /// Mutable access to the disabled skills snapshot.
    fn skill_picker_snapshot_mut(&mut self) -> &mut Option<HashSet<String>>;
    /// Current preview pane scroll offset for the skill picker.
    fn skill_preview_scroll(&self) -> usize;
    /// Set the preview pane scroll offset for the skill picker.
    fn set_skill_preview_scroll(&mut self, val: usize);

    // --- Session lifecycle picker ---

    /// Read-only access to the session lifecycle picker state.
    fn session_lifecycle_picker(&self) -> &jinn_selection_widget::SelectionState<SessionLifecycleEntry>;
    /// Mutable access to the session lifecycle picker state.
    fn session_lifecycle_picker_mut(&mut self) -> &mut jinn_selection_widget::SelectionState<SessionLifecycleEntry>;

    // --- Workflow picker ---

    /// Read-only access to the workflow picker state.
    fn workflow_picker(&self) -> &jinn_selection_widget::SelectionState<WorkflowPickerEntry>;
    /// Mutable access to the workflow picker state.
    fn workflow_picker_mut(&mut self) -> &mut jinn_selection_widget::SelectionState<WorkflowPickerEntry>;


    // --- Compaction model picker ---

    /// Read-only access to the compaction model picker state.
    fn compaction_model_picker(&self) -> &jinn_selection_widget::SelectionState<PickerEntry>;
    /// Mutable access to the compaction model picker state.
    fn compaction_model_picker_mut(&mut self) -> &mut jinn_selection_widget::SelectionState<PickerEntry>;
}

impl PickerExt for super::frontend_state::FrontendState {
    fn session_picker(&self) -> &jinn_selection_widget::TreePickerState<SessionTreeEntry> {
        &self.pickers.session_picker
    }

    fn session_picker_mut(&mut self) -> &mut jinn_selection_widget::TreePickerState<SessionTreeEntry> {
        &mut self.pickers.session_picker
    }

    fn persona_picker(&self) -> &jinn_selection_widget::SelectionState<PersonaEntry> {
        &self.pickers.persona_picker
    }

    fn persona_picker_mut(&mut self) -> &mut jinn_selection_widget::SelectionState<PersonaEntry> {
        &mut self.pickers.persona_picker
    }

    fn theme_picker(&self) -> &jinn_selection_widget::SelectionState<ThemeEntry> {
        &self.pickers.theme_picker
    }

    fn theme_picker_mut(&mut self) -> &mut jinn_selection_widget::SelectionState<ThemeEntry> {
        &mut self.pickers.theme_picker
    }

    fn theme_preview_original(&self) -> &Option<Theme> {
        &self.pickers.theme_preview_original
    }

    fn theme_preview_original_mut(&mut self) -> &mut Option<Theme> {
        &mut self.pickers.theme_preview_original
    }

    fn tool_picker(&self) -> &jinn_selection_widget::SelectionState<ToolEntry> {
        &self.pickers.tool_picker
    }

    fn tool_picker_mut(&mut self) -> &mut jinn_selection_widget::SelectionState<ToolEntry> {
        &mut self.pickers.tool_picker
    }

    fn tool_picker_snapshot(&self) -> &Option<HashSet<String>> {
        &self.pickers.tool_picker_snapshot
    }

    fn tool_picker_snapshot_mut(&mut self) -> &mut Option<HashSet<String>> {
        &mut self.pickers.tool_picker_snapshot
    }

    fn skill_picker(&self) -> &jinn_selection_widget::SelectionState<SkillEntry> {
        &self.pickers.skill_picker
    }

    fn skill_picker_mut(&mut self) -> &mut jinn_selection_widget::SelectionState<SkillEntry> {
        &mut self.pickers.skill_picker
    }

    fn skill_picker_snapshot(&self) -> &Option<HashSet<String>> {
        &self.pickers.skill_picker_snapshot
    }

    fn skill_picker_snapshot_mut(&mut self) -> &mut Option<HashSet<String>> {
        &mut self.pickers.skill_picker_snapshot
    }

    fn skill_preview_scroll(&self) -> usize {
        self.pickers.skill_preview_scroll
    }

    fn set_skill_preview_scroll(&mut self, val: usize) {
        self.pickers.skill_preview_scroll = val;
    }

    fn session_lifecycle_picker(&self) -> &jinn_selection_widget::SelectionState<SessionLifecycleEntry> {
        &self.pickers.session_lifecycle_picker
    }

    fn session_lifecycle_picker_mut(&mut self) -> &mut jinn_selection_widget::SelectionState<SessionLifecycleEntry> {
        &mut self.pickers.session_lifecycle_picker
    }

    fn workflow_picker(&self) -> &jinn_selection_widget::SelectionState<WorkflowPickerEntry> {
        &self.pickers.workflow_picker
    }

    fn workflow_picker_mut(&mut self) -> &mut jinn_selection_widget::SelectionState<WorkflowPickerEntry> {
        &mut self.pickers.workflow_picker
    }


    fn compaction_model_picker(&self) -> &jinn_selection_widget::SelectionState<PickerEntry> {
        &self.pickers.compaction_model_picker
    }

    fn compaction_model_picker_mut(&mut self) -> &mut jinn_selection_widget::SelectionState<PickerEntry> {
        &mut self.pickers.compaction_model_picker
    }
}
