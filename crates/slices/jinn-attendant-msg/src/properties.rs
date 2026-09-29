//! Attendant properties popup — the edit state for its three fields.
//!
//! The popup is a two-phase vim-style form over an attendant's trigger,
//! activation, and seed template. `j`/`k` move the form cursor between
//! fields, `h`/`l` pick a choice within the focused field, and `i` opens
//! the seed-template editor on its own scope. Every edit stays pending —
//! the popup never touches the session — until the user applies all three
//! fields at once. The values the session had at open are snapshotted so
//! leaving restores them exactly.
//!
//! Choice rows carry their display order and labels here ([`TRIGGER_CHOICES`],
//! [`ACTIVATION_CHOICES`]) so the renderer and the pick helpers share one
//! source of truth; nothing downstream hardcodes a choice string.

use crate::{AttendantActivation, AttendantTrigger};
use jinn_slices::LineInput;
use jinn_slices::SlotKey;

/// The `attendant/properties` slot: the properties popup's single edit state.
///
/// The seed-template editor shares this slot — it is only ever open while a
/// properties edit is pending, so a second cell would model nothing.
#[must_use]
pub fn attendant_properties_slot() -> SlotKey {
    SlotKey::builtin("attendant", "properties")
}

/// The properties popup's scope: navigation-only.
///
/// `j`/`k`/`h`/`l`/`i` drive the form. The scope captures no input — typed
/// characters never land here; the seed template edits through
/// [`attendant_seed_template_scope`].
#[must_use]
pub fn attendant_properties_scope() -> jinn_slices::slice_scope::SliceScopeId {
    jinn_slices::slice_scope::SliceScopeId::navigation("attendant", "properties")
}

/// The seed-template editor's scope: input-capturing, rename-popup parity.
///
/// Pushed by the properties popup's `i` row and popped by the editor's
/// keep/restore/clear rows. Capturing is what generates the editing keys
/// and the printable-character catch-all for the editor's input hook.
#[must_use]
pub fn attendant_seed_template_scope() -> jinn_slices::slice_scope::SliceScopeId {
    jinn_slices::slice_scope::SliceScopeId::new("attendant", "seed-template")
}

/// The trigger's choices in display order: first shown leftmost.
pub const TRIGGER_CHOICES: &[(AttendantTrigger, &str)] = &[
    (AttendantTrigger::ParentCompleted, "parent-completed"),
    (AttendantTrigger::Manual, "manual"),
];

/// The activation's choices in display order: first shown leftmost.
pub const ACTIVATION_CHOICES: &[(AttendantActivation, &str)] = &[
    (AttendantActivation::Seed, "seed"),
    (AttendantActivation::Reset, "reset"),
    (AttendantActivation::Preserve, "preserve"),
];

/// One field of the properties form, in display order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PropertyField {
    /// When the attendant re-runs. A choice field (`h`/`l`).
    #[default]
    Trigger,
    /// How the attendant's context is prepared per run. A choice field.
    Activation,
    /// The seed text, edited through the template editor (`i`).
    SeedTemplate,
}

impl PropertyField {
    /// The next field in display order, stopping at the last.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Trigger => Self::Activation,
            Self::Activation | Self::SeedTemplate => Self::SeedTemplate,
        }
    }

    /// The previous field in display order, stopping at the first.
    #[must_use]
    pub fn previous(self) -> Self {
        match self {
            Self::Trigger | Self::Activation => Self::Trigger,
            Self::SeedTemplate => Self::Activation,
        }
    }

    /// The field's label as shown in the popup.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Trigger => "trigger",
            Self::Activation => "activation",
            Self::SeedTemplate => "seed template",
        }
    }
}

/// Which way an `h`/`l` pick moves through a choice row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickDirection {
    /// `h` — toward the first (leftmost) choice.
    Left,
    /// `l` — toward the last (rightmost) choice.
    Right,
}

/// Picks the choice adjacent to `current`, clamped at the row's ends.
///
/// Derives the position from the value itself, so the pending value is the
/// only stored state and cannot desync from an index.
fn pick_choice<'a, T>(
    choices: &'a [(T, &'static str)],
    current: &T,
    direction: PickDirection,
) -> Option<&'a T>
where
    T: PartialEq,
{
    if choices.is_empty() {
        return None;
    }
    let index = choices.iter().position(|(value, _)| value == current)?;
    let next = match direction {
        PickDirection::Left => index.saturating_sub(1),
        PickDirection::Right => (index + 1).min(choices.len() - 1),
    };
    choices.get(next).map(|(value, _)| value)
}

/// Picks the adjacent trigger choice, clamped at the row's ends.
#[must_use]
pub fn pick_trigger(current: AttendantTrigger, direction: PickDirection) -> AttendantTrigger {
    pick_choice(TRIGGER_CHOICES, &current, direction)
        .copied()
        .unwrap_or(current)
}

/// Picks the adjacent activation choice, clamped at the row's ends.
#[must_use]
pub fn pick_activation(
    current: AttendantActivation,
    direction: PickDirection,
) -> AttendantActivation {
    pick_choice(ACTIVATION_CHOICES, &current, direction)
        .copied()
        .unwrap_or(current)
}

/// The values an attendant had when the properties popup opened.
///
/// Leaving the popup restores these; applying replaces them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginalValues {
    /// The trigger at open time.
    pub trigger: AttendantTrigger,
    /// The activation at open time.
    pub activation: AttendantActivation,
    /// The seed template at open time.
    pub template: String,
}

/// State for the attendant properties popup.
///
/// Opened from the sessions section with `P`; all three fields target the
/// highlighted attendant session. Nothing here reaches the session — the
/// popup holds pending edits until the apply row commits them together.
#[derive(Debug, Clone, Default)]
pub struct AttendantPropertiesState {
    /// The session the popup is editing. `None` while the popup is closed.
    pub session_id: Option<jinn_core_types::SessionId>,
    /// The field the form cursor is on (`j`/`k`).
    pub focus: PropertyField,
    /// The activation `<enter>` would apply right now.
    pub pending_activation: AttendantActivation,
    /// The trigger `<enter>` would apply right now.
    pub pending_trigger: AttendantTrigger,
    /// The values the session had at open; `<esc>`/`<c-c>` restore them.
    /// `None` while the popup is closed.
    pub original: Option<OriginalValues>,
    /// The editable seed-template text + cursor.
    pub seed_template: LineInput,
    /// The template text captured when `i` opened the editor; `<esc>` in
    /// the editor restores it. `None` while the editor is closed.
    pub editor_original: Option<String>,
    /// Whether the help overlay is showing. Toggled with `?`; it targets
    /// whichever field the form cursor is on, so it is never stale.
    pub help_visible: bool,
    /// Whether a second `<c-s>` would overwrite a same-named saved
    /// attendant.
    ///
    /// Saving over an existing entry destroys whatever that entry
    /// currently holds, so it is armed by the first stroke and committed by
    /// the second; a first save of a new name needs no second stroke,
    /// because there is nothing to destroy. The flag is popup-local and
    /// cleared on every path that leaves the popup (apply, `<esc>`,
    /// `<c-c>`, a committed save), so an armed popup can never leak an
    /// armed state into the next one.
    pub save_armed: bool,
    /// The popup's one-line status, shown under the form.
    ///
    /// Holds what the last key did: a save that wrote, a save that was
    /// refused, an overwrite waiting on its second stroke. The form has no
    /// other way to answer a key — the session is only written by
    /// `<enter>`, and a save that armed would otherwise be invisible.
    ///
    /// Cleared by the next keystroke, so the line always describes the most
    /// recent thing that happened rather than accumulating a history of
    /// things that did.
    pub status: Option<PopupStatus>,
}

/// What the popup's status line reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopupStatus {
    /// An overwrite is armed; the next `<c-s>` replaces the entry.
    OverwriteArmed {
        /// The name whose entry is about to be replaced.
        name: String,
    },
    /// A save wrote the attendant to `jinn.toml`.
    Saved {
        /// The name the attendant was saved under.
        name: String,
    },
    /// A save did not happen, and this is why.
    SaveFailed {
        /// The reason, in the user's terms.
        reason: String,
    },
}

impl AttendantPropertiesState {
    /// Moves the form cursor to the next field, stopping at the last.
    pub fn focus_next(&mut self) {
        self.focus = self.focus.next();
    }

    /// Moves the form cursor to the previous field, stopping at the first.
    pub fn focus_previous(&mut self) {
        self.focus = self.focus.previous();
    }

    /// Picks the adjacent choice in the focused field (`h`/`l`).
    ///
    /// Clamps at the row's ends. A no-op on the seed-template field: it is
    /// not a choice row — it edits through the template editor.
    pub fn pick(&mut self, direction: PickDirection) {
        match self.focus {
            PropertyField::Trigger => {
                self.pending_trigger = pick_trigger(self.pending_trigger, direction);
            }
            PropertyField::Activation => {
                self.pending_activation = pick_activation(self.pending_activation, direction);
            }
            PropertyField::SeedTemplate => {}
        }
    }

    /// Restores every field to the values captured at open, discarding all
    /// pending edits and any editor draft.
    ///
    /// A no-op without a snapshot (the popup was never opened). Also
    /// disarms the save: this is the `<esc>`/`<c-c>` close path, and an
    /// armed overwrite must not survive into the next popup session.
    pub fn restore_original(&mut self) {
        self.save_armed = false;
        let Some(original) = self.original.clone() else {
            return;
        };
        let cursor_pos = original.template.len();
        self.pending_activation = original.activation;
        self.pending_trigger = original.trigger;
        self.seed_template = LineInput {
            input: original.template,
            cursor_pos,
        };
        self.editor_original = None;
    }

    /// Captures the current template text as the editor's restore point.
    pub fn begin_template_edit(&mut self) {
        self.editor_original = Some(self.seed_template.input.clone());
    }

    /// Restores the pre-editor template text and closes the editor state.
    pub fn cancel_template_edit(&mut self) {
        if let Some(original) = self.editor_original.take() {
            let cursor_pos = original.len();
            self.seed_template = LineInput {
                input: original,
                cursor_pos,
            };
        }
    }

    /// Accepts the edited text and closes the editor state.
    pub fn keep_template_edit(&mut self) {
        self.editor_original = None;
    }

    /// Arms an overwrite of the named saved attendant.
    ///
    /// The arming is what a second `<c-s>` confirms: the entry that exists
    /// under this name is about to be replaced by this session's
    /// configuration, and a user who did not mean to replace it can still
    /// change the name before pressing again.
    pub fn arm_save(&mut self) {
        self.save_armed = true;
    }

    /// Disarms the overwrite, after a committed save.
    pub fn disarm_save(&mut self) {
        self.save_armed = false;
    }

    /// Replaces the status line's message.
    pub fn report(&mut self, status: PopupStatus) {
        self.status = Some(status);
    }

    /// Clears the status line, and disarms an armed overwrite.
    ///
    /// Called on every keystroke that is not itself a status-producing one:
    /// the line describes the most recent key, so a message about an armed
    /// overwrite must not still be claiming the next `<c-s>` is waiting
    /// after the user has moved on.
    ///
    /// The arm goes with the message. Hiding the prompt while leaving the
    /// save armed is the one sequence that silently destroys an entry: the
    /// user presses some other key, the "Overwrite …?" line disappears, and
    /// the `<c-s>` they press next — the one they were told to press to
    /// confirm — overwrites with no confirmation ever having been on
    /// screen. The prompt and the arm are one piece of state: the save
    /// that follows an arming is the save the user was shown, so
    /// everything that hides the prompt also withdraws the offer.
    pub fn clear_status(&mut self) {
        self.status = None;
        self.save_armed = false;
    }
}
