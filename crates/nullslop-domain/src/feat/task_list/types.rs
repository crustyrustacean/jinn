//! Task list data model — phases, tasks, and positioning.
//!
//! A [`TaskList`] contains ordered [`Phase`]s, each containing ordered [`Task`]s.
//! One level of nesting: phases are containers, tasks are leaf items.
//!
//! IDs are stable auto-incrementing strings ("p1", "p2" for phases; "t1", "t2" for tasks).
//! They never change even if items are reordered.

use std::fmt;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ID types
// ---------------------------------------------------------------------------

/// Unique identifier for a phase within a task list.
///
/// Generated as "p1", "p2", etc. Stable — never changes after creation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PhaseId(String);

impl PhaseId {
    fn new(counter: u64) -> Self {
        Self(format!("p{counter}"))
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
/// Generated as "t1", "t2", etc. Globally unique across all phases.
/// Stable — never changes after creation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(String);

impl TaskId {
    fn new(counter: u64) -> Self {
        Self(format!("t{counter}"))
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
    /// Task is pending — not yet done.
    #[default]
    Pending,
    /// Task has been completed.
    Completed,
}

impl TaskStatus {
    /// Returns the display indicator for this status.
    pub fn indicator(&self) -> &'static str {
        match self {
            Self::Pending => "○",
            Self::Completed => "✓",
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
        /// The task that was referenced.
        task_id: TaskId,
        /// The phase that was targeted.
        phase_id: PhaseId,
    },
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

/// A phase — a named container of ordered tasks.
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

/// A phased task list — the top-level container for agent planning.
///
/// Contains ordered phases, each containing ordered tasks.
/// Stored per-session on [`SessionCore`](crate::feat::session::chat_session::SessionCore).
///
/// # Persistence
///
/// Derives `Serialize`/`Deserialize` — the existing session save/load pipeline
/// handles persistence automatically. The `#[serde(default)]` attribute on the
/// `SessionCore` field ensures backward compatibility with old sessions.
/// Serde default for counter fields — starts at 1 so the first ID is "p1"/"t1".
const fn default_counter_start() -> u64 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskList {
    /// Ordered phases in this task list.
    #[serde(default)]
    pub(crate) phases: Vec<Phase>,
    /// Auto-increment counter for phase IDs. Starts at 1.
    #[serde(default = "default_counter_start")]
    next_phase_id: u64,
    /// Auto-increment counter for task IDs (global across all phases). Starts at 1.
    #[serde(default = "default_counter_start")]
    next_task_id: u64,
}

impl Default for TaskList {
    fn default() -> Self {
        Self {
            phases: Vec::new(),
            next_phase_id: 1,
            next_task_id: 1,
        }
    }
}

impl TaskList {
    /// Creates a new empty task list.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a new phase and returns its ID.
    pub fn add_phase(&mut self, description: &str) -> PhaseId {
        let id = PhaseId::new(self.next_phase_id);
        self.next_phase_id += 1;
        self.phases.push(Phase {
            id: id.clone(),
            description: description.to_owned(),
            tasks: Vec::new(),
        });
        id
    }

    /// Adds a new task to the specified phase at the given position.
    ///
    /// Returns the new task's ID, or an error if the phase or position reference is invalid.
    pub fn add_task(
        &mut self,
        phase_id: &PhaseId,
        description: &str,
        position: TaskPosition,
    ) -> Result<TaskId, TaskListError> {
        let task_id = TaskId::new(self.next_task_id);
        self.next_task_id += 1;

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
                let idx = phase
                    .find_task_index(&after_id)
                    .ok_or_else(|| TaskListError::TaskNotInPhase {
                        task_id: after_id.clone(),
                        phase_id: phase_id.clone(),
                    })?;
                phase.tasks.insert(idx + 1, new_task);
            }
            TaskPosition::Before(before_id) => {
                let idx = phase
                    .find_task_index(&before_id)
                    .ok_or_else(|| TaskListError::TaskNotInPhase {
                        task_id: before_id.clone(),
                        phase_id: phase_id.clone(),
                    })?;
                phase.tasks.insert(idx, new_task);
            }
        }

        Ok(task_id)
    }

    /// Marks a task as completed.
    ///
    /// Searches all phases for the task. Returns an error if not found.
    pub fn complete_task(&mut self, task_id: &TaskId) -> Result<(), TaskListError> {
        for phase in &mut self.phases {
            if let Some(task) = phase.tasks.iter_mut().find(|t| &t.id == task_id) {
                task.status = TaskStatus::Completed;
                return Ok(());
            }
        }
        Err(TaskListError::TaskNotFound(task_id.clone()))
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
            lines.push(format!("## Phase {}: {} [{}]", i + 1, phase.description, phase.id));
            if phase.tasks.is_empty() {
                lines.push("  (no tasks)".to_owned());
            } else {
                for task in &phase.tasks {
                    let check = match task.status {
                        TaskStatus::Pending => " ",
                        TaskStatus::Completed => "✓",
                    };
                    lines.push(format!("- [{}] {} [{}]", check, task.description, task.id));
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
            for task in &phase.tasks {
                let check = match task.status {
                    TaskStatus::Pending => " ",
                    TaskStatus::Completed => "✓",
                };
                lines.push(format!("- [{}] {} [{}]", check, task.description, task.id));
            }
        }

        Some(lines.join("\n"))
    }
}
