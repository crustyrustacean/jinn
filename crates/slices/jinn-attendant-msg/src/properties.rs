//! Attendant properties popup — the edit state for its four fields.
//!
//! The popup is a two-phase vim-style form over an attendant's trigger,
//! behavior, prep mode, and seed template. `j`/`k` move the form cursor
//! between fields, `h`/`l` act within the focused field, and `i` opens the
//! seed-template editor on its own scope. Every edit stays pending — the
//! popup never touches the session — until the user applies all four fields
//! at once. The values the session had at open are snapshotted so leaving
//! restores them exactly.
//!
//! Prep mode cages the two rows above it. While the attendant is being
//! composed nothing runs, so its trigger and behavior are values the user
//! has written but that do not apply; they stay on screen, dimmed, and the
//! cursor cannot reach them, which is a stronger statement than letting the
//! cursor arrive and having the pick do nothing.
//!
//! Choice rows carry their display order and labels here ([`TRIGGER_CHOICES`],
//! [`BEHAVIOR_CHOICES`]) so the renderer and the pick helpers share one
//! source of truth; nothing downstream hardcodes a choice string.

use crate::{AttendantBehavior, AttendantTrigger};
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

/// The behavior's choices in display order: first shown leftmost.
pub const BEHAVIOR_CHOICES: &[(AttendantBehavior, &str)] = &[
    (AttendantBehavior::Reset, "reset"),
    (AttendantBehavior::Preserve, "preserve"),
];

/// One field of the properties form, in display order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PropertyField {
    /// When the attendant re-runs on its own. A choice field (`h`/`l`).
    #[default]
    Trigger,
    /// What the attendant's context looks like per run. A choice field.
    Behavior,
    /// Whether the attendant is still being composed. A two-state field:
    /// `h`/`l` cycle it, and while it is on, the two rows above do not
    /// apply.
    PrepMode,
    /// The seed text, edited through the template editor (`i`).
    SeedTemplate,
}

impl PropertyField {
    /// The next field in display order, stopping at the last.
    ///
    /// While the attendant is being composed the two rows above the prep
    /// row are unreachable, so moving *up* off the prep row stays put
    /// rather than landing on a row that governs nothing. The rows are
    /// still rendered — a hidden row is not a disabled one, and the value
    /// the user wrote has to be visible to be worth keeping.
    #[must_use]
    pub fn next(self, prep_mode: bool) -> Self {
        let stepped = match self {
            Self::Trigger => Self::Behavior,
            Self::Behavior => Self::PrepMode,
            Self::PrepMode | Self::SeedTemplate => Self::SeedTemplate,
        };
        match stepped {
            Self::Trigger | Self::Behavior if prep_mode => self,
            _ => stepped,
        }
    }

    /// The previous field in display order, stopping at the first reachable.
    ///
    /// The step is the plain one row up, then clamped to the first row the
    /// cursor may rest on. While the attendant is being composed that floor
    /// is the prep row, so moving up off it stays put rather than landing on
    /// one of the two rows that do not apply. Clamping *after* stepping is
    /// what keeps a single `k` meaning "the row above me" — computing the
    /// target from the floor instead would make one press skip a row as
    /// soon as composition ended, because the floor had moved.
    #[must_use]
    pub fn previous(self, prep_mode: bool) -> Self {
        let floor = Self::first_reachable(prep_mode);
        match self {
            Self::Trigger => floor,
            Self::Behavior => Self::Trigger,
            Self::PrepMode => Self::Behavior,
            Self::SeedTemplate => Self::PrepMode,
        }
        .clamp_to(floor)
    }

    /// The nearest field at or above this one that the cursor may rest on.
    fn clamp_to(self, floor: Self) -> Self {
        if rank(self) < rank(floor) {
            floor
        } else {
            self
        }
    }

    /// The first field the cursor may rest on, given the prep state.
    #[must_use]
    pub fn first_reachable(prep_mode: bool) -> Self {
        if prep_mode {
            Self::PrepMode
        } else {
            Self::Trigger
        }
    }

    /// The field the form cursor rests on when the popup opens.
    ///
    /// Prep mode is where a composing attendant's cursor belongs: it is the
    /// one row above which two inapplicable rows sit, so it is the only row
    /// a user looking at a fresh attendant can usefully act on. An attendant
    /// that is not being composed opens on the first row as before.
    #[must_use]
    pub fn opening_focus(prep_mode: bool) -> Self {
        Self::first_reachable(prep_mode)
    }

    /// Whether this field applies while the attendant is being composed.
    ///
    /// The seed template survives: pins are the whole point of composing,
    /// so a template written during prep is exactly what the first run
    /// should inject.
    #[must_use]
    pub fn applies_while_prepping(self) -> bool {
        matches!(self, Self::PrepMode | Self::SeedTemplate)
    }

    /// The field's label as shown in the popup.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Trigger => "trigger",
            Self::Behavior => "behavior",
            Self::PrepMode => "prep mode",
            Self::SeedTemplate => "seed template",
        }
    }
}

/// A field's position in display order, for comparing two of them.
///
/// Declared after the enum rather than as a `rank` on it because a field's
/// place in the *form* is the form's business: the enum names four settings,
/// and nothing about the settings themselves says which row they sit on.
fn rank(field: PropertyField) -> u8 {
    match field {
        PropertyField::Trigger => 0,
        PropertyField::Behavior => 1,
        PropertyField::PrepMode => 2,
        PropertyField::SeedTemplate => 3,
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

/// Picks the adjacent behavior choice, clamped at the row's ends.
#[must_use]
pub fn pick_behavior(current: AttendantBehavior, direction: PickDirection) -> AttendantBehavior {
    pick_choice(BEHAVIOR_CHOICES, &current, direction)
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
    /// The behavior at open time.
    pub behavior: AttendantBehavior,
    /// Whether the attendant was being composed at open time.
    pub prep_mode: bool,
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
    /// What the behavior `<enter>` would apply right now.
    pub pending_behavior: AttendantBehavior,
    /// The trigger `<enter>` would apply right now.
    pub pending_trigger: AttendantTrigger,
    /// Whether `<enter>` would leave the attendant in prep mode.
    pub pending_prep_mode: bool,
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
    ///
    /// While the attendant is being composed the cursor cannot pass up
    /// through the prep row, so the two rows above are unreachable.
    pub fn focus_next(&mut self) {
        self.focus = self.focus.next(self.pending_prep_mode);
    }

    /// Moves the form cursor to the previous field, stopping at the first.
    pub fn focus_previous(&mut self) {
        self.focus = self.focus.previous(self.pending_prep_mode);
    }

    /// Acts on the focused field: picks a choice, or cycles prep mode.
    ///
    /// `h` and `l` are the same key on a two-state field — there is no
    /// previous or next, only the other value — so a two-state cycle is
    /// what the caller gets whichever key it pressed.
    ///
    /// A no-op on the seed-template field: it is not a choice row, it edits
    /// through the template editor. A no-op on a choice row while the
    /// attendant is being composed, which the cage already makes
    /// unreachable; the guard is here so a caller that reaches the row
    /// some other way cannot move a value that governs nothing.
    pub fn pick(&mut self, direction: PickDirection) {
        match self.focus {
            PropertyField::Trigger if !self.pending_prep_mode => {
                self.pending_trigger = pick_trigger(self.pending_trigger, direction);
            }
            PropertyField::Behavior if !self.pending_prep_mode => {
                self.pending_behavior = pick_behavior(self.pending_behavior, direction);
            }
            PropertyField::PrepMode => {
                self.pending_prep_mode = !self.pending_prep_mode;
            }
            PropertyField::Trigger | PropertyField::Behavior | PropertyField::SeedTemplate => {}
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
        self.pending_behavior = original.behavior;
        self.pending_trigger = original.trigger;
        self.pending_prep_mode = original.prep_mode;
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
