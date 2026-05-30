//! Todo list subsystem - phased task tracking for agent sessions.
//!
//! Provides a structured task list with one level of nesting: phases contain tasks.
//! The data model is stored per-session on [`SessionCore`](crate::feat::session::chat_session::SessionCore)
//! and persists across restarts via the existing session serialization pipeline.

pub mod tools;

#[cfg(test)]
mod types_tests;

use std::fmt;

use rand::Rng;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ID generation
// ---------------------------------------------------------------------------

/// Characters used for random ID generation: a-z and 0-9, excluding `p` and `t`.
///
/// `p` and `t` are excluded to avoid ambiguity with the phase/task ID prefixes.
/// 34 characters → 34³ = 39,304 possible IDs per type.
const ID_CHARSET: &[u8] = b"abcdefghijklmnqrsuvwxyz0123456789";

/// Generates 3 random characters from the ID charset.
fn generate_id_chars() -> [u8; 3] {
    let mut rng = rand::rng();
    [
        ID_CHARSET[rng.random_range(0..ID_CHARSET.len())],
        ID_CHARSET[rng.random_range(0..ID_CHARSET.len())],
        ID_CHARSET[rng.random_range(0..ID_CHARSET.len())],
    ]
}

// ---------------------------------------------------------------------------
// ID types
// ---------------------------------------------------------------------------

/// Unique identifier for a phase within a task list.
///
/// Random 3-char alphanumeric (excluding `p` and `t`) prefixed with `p`.
/// Globally unique within the task list - collision-checked against existing IDs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct PhaseId(String);

impl PhaseId {
    fn new(existing: &[PhaseId]) -> Self {
        loop {
            let chars = generate_id_chars();
            let candidate =
                format!("p{}", std::str::from_utf8(&chars).expect("charset is valid UTF-8"));
            if !existing.iter().any(|e| e.0 == candidate) {
                return Self(candidate);
            }
        }
    }

    /// Creates a PhaseId from a known string.
    ///
    /// Used by tools to reconstruct a PhaseId from an LLM-provided string.
    pub(crate) fn from_string(s: String) -> Self {
        Self(s)
    }

    /// Creates a PhaseId from a known string (for testing).
    #[cfg(test)]
    pub(crate) fn new_for_test(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl fmt::Display for PhaseId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for PhaseId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Unique identifier for a task within a task list.
///
/// Random 3-char alphanumeric (excluding `p` and `t`) prefixed with `t`.
/// Globally unique across all phases - collision-checked against existing IDs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct TaskId(String);

impl TaskId {
    fn new(existing: &[TaskId]) -> Self {
        loop {
            let chars = generate_id_chars();
            let candidate =
                format!("t{}", std::str::from_utf8(&chars).expect("charset is valid UTF-8"));
            if !existing.iter().any(|e| e.0 == candidate) {
                return Self(candidate);
            }
        }
    }

    /// Creates a TaskId from a known string.
    ///
    /// Used by tools to reconstruct a TaskId from an LLM-provided string.
    pub(crate) fn from_string(s: String) -> Self {
        Self(s)
    }

    /// Creates a TaskId from a known string (for testing).
    #[cfg(test)]
    pub(crate) fn new_for_test(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for TaskId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// ---------------------------------------------------------------------------
// Task status
// ---------------------------------------------------------------------------

/// The status of a task item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TaskStatus {
    /// Task is pending - not yet done.
    #[default]
    Pending,
    /// Task has been completed.
    Completed,
    /// Task has been deferred to a later phase.
    Deferred,
}

impl TaskStatus {
    /// Returns the display indicator for this status.
    pub fn indicator(&self) -> &'static str {
        match self {
            Self::Pending => "\u{25CB}",
            Self::Completed => "\u{2713}",
            Self::Deferred => "\u{25BC}",
        }
    }
}

// ---------------------------------------------------------------------------
// Positioning
// ---------------------------------------------------------------------------

/// Where to insert a new task within a phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskPosition {
    /// Append to the end of the phase.
    End,
    /// Insert immediately after the specified task.
    After(TaskId),
    /// Insert immediately before the specified task.
    Before(TaskId),
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur when mutating a task list.
#[derive(Debug, Clone, PartialEq, Eq, wherror::Error)]
pub enum TaskListError {
    /// The specified phase does not exist.
    #[error("phase not found: {0}")]
    PhaseNotFound(PhaseId),
    /// The specified task does not exist.
    #[error("task not found: {0}")]
    TaskNotFound(TaskId),
    /// Both `after` and `before` were specified (mutually exclusive).
    #[error("cannot specify both after_task and before_task")]
    BothAfterAndBefore,
    /// The referenced task belongs to a different phase.
    #[error("task {task_id} is not in phase {phase_id}")]
    TaskNotInPhase {
        task_id: TaskId,
        phase_id: PhaseId,
    },
    /// Cannot defer a task relative to itself.
    #[error("cannot defer task relative to itself: {0}")]
    SelfReference(TaskId),
    /// The task is already deferred.
    #[error("task is already deferred: {0}")]
    AlreadyDeferred(TaskId),
    /// The phases list provided was empty.
    #[error("phases list must not be empty")]
    EmptyPhasesList,
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

/// A single task item within a phase.
///
/// Tasks have a stable ID, a description, and a completion status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Unique identifier for this task.
    pub(crate) id: TaskId,
    /// Human-readable description of the task.
    pub(crate) description: String,
    /// Current status of the task.
    pub(crate) status: TaskStatus,
}

impl Task {
    /// Returns this task's unique identifier.
    pub fn id(&self) -> &TaskId {
        &self.id
    }

    /// Returns this task's description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns this task's current status.
    pub fn status(&self) -> TaskStatus {
        self.status
    }
}

// ---------------------------------------------------------------------------
// Phase
// ---------------------------------------------------------------------------

/// A phase - a named container of ordered tasks.
///
/// Phases represent high-level stages of work (e.g., "Research", "Build", "Test").
/// Tasks within a phase are ordered and can be repositioned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phase {
    /// Unique identifier for this phase.
    pub(crate) id: PhaseId,
    /// Human-readable description of the phase.
    pub(crate) description: String,
    /// Ordered tasks within this phase.
    pub(crate) tasks: Vec<Task>,
}

impl Phase {
    /// Returns this phase's unique identifier.
    pub fn id(&self) -> &PhaseId {
        &self.id
    }

    /// Returns this phase's description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns the tasks in this phase.
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// Returns true if this phase has no tasks.
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Finds the index of a task by ID within this phase.
    pub(crate) fn find_task_index(&self, task_id: &TaskId) -> Option<usize> {
        self.tasks.iter().position(|t| &t.id == task_id)
    }
}

// ---------------------------------------------------------------------------
// TaskList
// ---------------------------------------------------------------------------

/// A phased task list - the top-level container for agent planning.
///
/// Contains ordered phases, each containing ordered tasks.
/// Stored per-session on [`SessionCore`](crate::feat::session::chat_session::SessionCore).
///
/// # Persistence
///
/// Derives `Serialize`/`Deserialize` - the existing session save/load pipeline
/// handles persistence automatically. The `#[serde(default)]` attribute on the
/// `SessionCore` field ensures backward compatibility with old sessions.
/// Old serialized data with counter fields (`next_phase_id`, `next_task_id`) will
/// deserialize cleanly - unknown fields are ignored by serde's default behavior.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskList {
    /// Ordered phases in this task list.
    #[serde(default)]
    pub(crate) phases: Vec<Phase>,
}

impl TaskList {
    /// Creates a new empty task list.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a new phase and returns its ID.
    pub fn add_phase(&mut self, description: &str) -> PhaseId {
        let existing: Vec<_> = self.phases.iter().map(|p| p.id.clone()).collect();
        let id = PhaseId::new(&existing);
        self.phases.push(Phase {
            id: id.clone(),
            description: description.to_owned(),
            tasks: Vec::new(),
        });
        id
    }

    /// Adds a new task to the specified phase at the given position.
    ///
    /// # Errors
    ///
    /// Returns the new task's ID, or an error if the phase or position reference is invalid.
    pub fn add_task(
        &mut self,
        phase_id: &PhaseId,
        description: &str,
        position: TaskPosition,
    ) -> Result<TaskId, TaskListError> {
        let existing: Vec<_> = self
            .phases
            .iter()
            .flat_map(|p| &p.tasks)
            .map(|t| t.id.clone())
            .collect();
        let task_id = TaskId::new(&existing);

        let new_task = Task {
            id: task_id.clone(),
            description: description.to_owned(),
            status: TaskStatus::Pending,
        };

        let phase = self
            .phases
            .iter_mut()
            .find(|p| &p.id == phase_id)
            .ok_or_else(|| TaskListError::PhaseNotFound(phase_id.clone()))?;

        match position {
            TaskPosition::End => {
                phase.tasks.push(new_task);
            }
            TaskPosition::After(after_id) => {
                let idx = phase.find_task_index(&after_id).ok_or_else(|| {
                    TaskListError::TaskNotInPhase {
                        task_id: after_id.clone(),
                        phase_id: phase_id.clone(),
                    }
                })?;
                phase.tasks.insert(idx + 1, new_task);
            }
            TaskPosition::Before(before_id) => {
                let idx = phase.find_task_index(&before_id).ok_or_else(|| {
                    TaskListError::TaskNotInPhase {
                        task_id: before_id.clone(),
                        phase_id: phase_id.clone(),
                    }
                })?;
                phase.tasks.insert(idx, new_task);
            }
        }

        Ok(task_id)
    }

    /// Marks a task as completed.
    ///
    /// Searches all phases for the task.
    ///
    /// # Errors
    ///
    /// Returns an error if not found.
    pub fn complete_task(&mut self, task_id: &TaskId) -> Result<(), TaskListError> {
        for phase in &mut self.phases {
            if let Some(task) = phase.tasks.iter_mut().find(|t| &t.id == task_id) {
                task.status = TaskStatus::Completed;
                return Ok(());
            }
        }
        Err(TaskListError::TaskNotFound(task_id.clone()))
    }

    /// Defers a task by marking it as deferred and creating a pending copy.
    ///
    /// The source task is marked with `Deferred` status (▼) and remains in place.
    /// A new `Pending` copy with the same description is created at the specified
    /// position relative to a reference task. The reference task determines the
    /// target phase.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The source task does not exist (`TaskNotFound`)
    /// - The source task is already deferred (`AlreadyDeferred`)
    /// - The reference task does not exist (`TaskNotFound`)
    /// - The source and reference are the same task (`SelfReference`)
    ///
    /// # Panics
    ///
    /// Panics if internal invariants are violated (source/reference found but then missing).
    #[allow(clippy::needless_pass_by_value)]
    pub fn defer_task(
        &mut self,
        source_task_id: &TaskId,
        position: TaskPosition,
    ) -> Result<TaskId, TaskListError> {
        // Extract the reference task ID from the position.
        let ref_task_id: &TaskId = match &position {
            TaskPosition::After(id) | TaskPosition::Before(id) => id,
            TaskPosition::End => {
                return Err(TaskListError::BothAfterAndBefore);
            }
        };

        // Validate source != reference.
        if source_task_id == ref_task_id {
            return Err(TaskListError::SelfReference(source_task_id.clone()));
        }

        // Find source task info (phase index, task index, description, status).
        let mut source_info: Option<(usize, usize, String, TaskStatus)> = None;
        for (pi, phase) in self.phases.iter().enumerate() {
            for (ti, task) in phase.tasks.iter().enumerate() {
                if &task.id == source_task_id {
                    source_info = Some((pi, ti, task.description.clone(), task.status));
                    break;
                }
            }
            if source_info.is_some() {
                break;
            }
        }

        let (src_pi, _src_ti, src_desc, src_status) =
            source_info.ok_or_else(|| TaskListError::TaskNotFound(source_task_id.clone()))?;

        // Validate source is not already deferred.
        if src_status == TaskStatus::Deferred {
            return Err(TaskListError::AlreadyDeferred(source_task_id.clone()));
        }

        // Find reference task's phase.
        let mut ref_phase_idx: Option<usize> = None;
        for (pi, phase) in self.phases.iter().enumerate() {
            if phase.tasks.iter().any(|t| &t.id == ref_task_id) {
                ref_phase_idx = Some(pi);
                break;
            }
        }

        let target_pi =
            ref_phase_idx.ok_or_else(|| TaskListError::TaskNotFound(ref_task_id.clone()))?;

        // Mark source task as deferred.
        self.phases[src_pi].tasks.iter_mut().find(|t| &t.id == source_task_id).expect("source was found above").status =
            TaskStatus::Deferred;

        // Generate new task ID.
        let existing: Vec<_> = self
            .phases
            .iter()
            .flat_map(|p| &p.tasks)
            .map(|t| t.id.clone())
            .collect();
        let new_task_id = TaskId::new(&existing);

        let new_task = Task {
            id: new_task_id.clone(),
            description: src_desc,
            status: TaskStatus::Pending,
        };

        // Insert at position in target phase.
        let phase = &mut self.phases[target_pi];
        match &position {
            TaskPosition::After(after_id) => {
                let idx = phase.find_task_index(after_id).expect("ref was found above");
                phase.tasks.insert(idx + 1, new_task);
            }
            TaskPosition::Before(before_id) => {
                let idx = phase.find_task_index(before_id).expect("ref was found above");
                phase.tasks.insert(idx, new_task);
            }
            TaskPosition::End => unreachable!(),
        }

        Ok(new_task_id)
    }

    /// Defers a task to the end of a specific phase (or a new phase).
    ///
    /// The source task is marked with `Deferred` status and remains in place.
    /// A new `Pending` copy with the same description is appended to the end
    /// of the target phase.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The source task does not exist (`TaskNotFound`)
    /// - The source task is already deferred (`AlreadyDeferred`)
    /// - `target_phase_id` is provided but the phase does not exist (`PhaseNotFound`)
    ///
    /// # Panics
    ///
    /// Panics if internal invariants are violated (source found but then missing).
    pub fn defer_to_phase(
        &mut self,
        source_task_id: &TaskId,
        target_phase_id: &PhaseId,
    ) -> Result<TaskId, TaskListError> {
        // Find source task info.
        let mut source_info: Option<(usize, String, TaskStatus)> = None;
        for phase in &self.phases {
            for task in &phase.tasks {
                if &task.id == source_task_id {
                    source_info = Some((phase.tasks.iter().position(|t| &t.id == source_task_id).unwrap(), task.description.clone(), task.status));
                    break;
                }
            }
            if source_info.is_some() {
                break;
            }
        }

        let (src_pi, src_desc, src_status) =
            source_info.ok_or_else(|| TaskListError::TaskNotFound(source_task_id.clone()))?;

        // Validate source is not already deferred.
        if src_status == TaskStatus::Deferred {
            return Err(TaskListError::AlreadyDeferred(source_task_id.clone()));
        }

        // Validate target phase exists.
        let target_pi = self
            .phases
            .iter()
            .position(|p| &p.id == target_phase_id)
            .ok_or_else(|| TaskListError::PhaseNotFound(target_phase_id.clone()))?;

        // Mark source task as deferred.
        self.phases[src_pi].tasks.iter_mut().find(|t| &t.id == source_task_id).expect("source was found above").status =
            TaskStatus::Deferred;

        // Generate new task ID.
        let existing: Vec<_> = self
            .phases
            .iter()
            .flat_map(|p| &p.tasks)
            .map(|t| t.id.clone())
            .collect();
        let new_task_id = TaskId::new(&existing);

        let new_task = Task {
            id: new_task_id.clone(),
            description: src_desc,
            status: TaskStatus::Pending,
        };

        // Append to end of target phase.
        self.phases[target_pi].tasks.push(new_task);

        Ok(new_task_id)
    }

    /// Replaces the entire task list from a description-based structure.
    ///
    /// Clears all existing phases and creates new ones from the provided data.
    /// Each tuple is `(phase_description, task_descriptions)`. All new tasks
    /// are created with `Pending` status.
    ///
    /// # Errors
    ///
    /// Returns an error if `phases` is empty.
    pub fn set_from_descriptions(
        &mut self,
        phases: Vec<(String, Vec<String>)>,
    ) -> Result<(), TaskListError> {
        if phases.is_empty() {
            return Err(TaskListError::EmptyPhasesList);
        }

        self.phases.clear();

        for (phase_desc, task_descs) in phases {
            let pid = self.add_phase(&phase_desc);
            for task_desc in task_descs {
                self.add_task(&pid, &task_desc, TaskPosition::End)?;
            }
        }

        Ok(())
    }

    /// Returns the phase with the given ID, if it exists.
    pub fn get_phase(&self, phase_id: &PhaseId) -> Option<&Phase> {
        self.phases.iter().find(|p| &p.id == phase_id)
    }

    /// Returns the task with the given ID, searching all phases.
    pub fn get_task(&self, task_id: &TaskId) -> Option<&Task> {
        self.phases
            .iter()
            .flat_map(|p| &p.tasks)
            .find(|t| &t.id == task_id)
    }

    /// Returns the ordered phases in this task list.
    pub fn phases(&self) -> &[Phase] {
        &self.phases
    }

    /// Returns true if the task list has no phases.
    pub fn is_empty(&self) -> bool {
        self.phases.is_empty()
    }

    /// Renders the task list as formatted markdown text.
    ///
    /// Used by tools to return the current state to the LLM.
    pub fn render_text(&self) -> String {
        if self.phases.is_empty() {
            return "No phases defined.".to_owned();
        }

        let mut lines = Vec::new();
        for (i, phase) in self.phases.iter().enumerate() {
            lines.push(format!(
                "## Phase {}: {} [{}]",
                i + 1,
                phase.description,
                phase.id
            ));
            if phase.tasks.is_empty() {
                lines.push("  (no tasks)".to_owned());
            } else {
                let visible: Vec<_> = phase
                    .tasks
                    .iter()
                    .filter(|t| t.status != TaskStatus::Deferred)
                    .collect();
                if visible.is_empty() {
                    lines.push("  (no tasks)".to_owned());
                } else {
                    for task in visible {
                        let check = match task.status {
                            TaskStatus::Pending => " ",
                            TaskStatus::Completed => "\u{2713}",
                            TaskStatus::Deferred => unreachable!(),
                        };
                        lines.push(format!("- [{}] {} [{}]", check, task.description, task.id));
                    }
                }
            }
            lines.push(String::new());
        }

        // Remove trailing newline.
        if lines.last() == Some(&String::new()) {
            lines.pop();
        }

        lines.join("\n")
    }

    /// Renders a single phase as formatted markdown text.
    pub fn render_phase_text(&self, phase_id: &PhaseId) -> Option<String> {
        let (i, phase) = self
            .phases
            .iter()
            .enumerate()
            .find(|(_, p)| &p.id == phase_id)?;

        let mut lines = vec![format!(
            "## Phase {}: {} [{}]",
            i + 1,
            phase.description,
            phase.id
        )];

        if phase.tasks.is_empty() {
            lines.push("  (no tasks)".to_owned());
        } else {
            let visible: Vec<_> = phase
                .tasks
                .iter()
                .filter(|t| t.status != TaskStatus::Deferred)
                .collect();
            if visible.is_empty() {
                lines.push("  (no tasks)".to_owned());
            } else {
                for task in visible {
                    let check = match task.status {
                        TaskStatus::Pending => " ",
                        TaskStatus::Completed => "\u{2713}",
                        TaskStatus::Deferred => unreachable!(),
                    };
                    lines.push(format!("- [{}] {} [{}]", check, task.description, task.id));
                }
            }
        }

        Some(lines.join("\n"))
    }
}
