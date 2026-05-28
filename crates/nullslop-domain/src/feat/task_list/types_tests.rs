//! Tests for the task list data model.
//!
//! BDD-style tests following AGENTS.md conventions.
//! Each test covers a single behavior.

use crate::feat::task_list::{PhaseId, TaskList, TaskListError, TaskPosition, TaskStatus};

// ---------------------------------------------------------------------------
// add_phase
// ---------------------------------------------------------------------------

#[test]
fn add_phase_creates_phase_with_id_and_description() {
    let mut list = TaskList::new();
    let id = list.add_phase("Research");
    assert_eq!(id, PhaseId::new_for_test("p1"));
    let phase = list.get_phase(&id).unwrap();
    assert_eq!(phase.description(), "Research");
    assert!(phase.is_empty());
}

#[test]
fn add_phase_increments_id_counter() {
    let mut list = TaskList::new();
    let id1 = list.add_phase("Research");
    let id2 = list.add_phase("Build");
    let id3 = list.add_phase("Test");
    assert_eq!(id1, PhaseId::new_for_test("p1"));
    assert_eq!(id2, PhaseId::new_for_test("p2"));
    assert_eq!(id3, PhaseId::new_for_test("p3"));
}

#[test]
fn add_phase_returns_distinct_ids() {
    let mut list = TaskList::new();
    let id1 = list.add_phase("A");
    let id2 = list.add_phase("B");
    assert_ne!(id1, id2);
}

// ---------------------------------------------------------------------------
// add_task — append (no position)
// ---------------------------------------------------------------------------

#[test]
fn add_task_appends_to_phase_end_when_no_position() {
    let mut list = TaskList::new();
    let pid = list.add_phase("Research");
    let t1 = list.add_task(&pid, "Read docs", TaskPosition::End).unwrap();
    let t2 = list.add_task(&pid, "Call API", TaskPosition::End).unwrap();
    let phase = list.get_phase(&pid).unwrap();
    assert_eq!(phase.tasks().len(), 2);
    assert_eq!(&phase.tasks()[0].id, &t1);
    assert_eq!(&phase.tasks()[1].id, &t2);
}

// ---------------------------------------------------------------------------
// add_task — insert_after
// ---------------------------------------------------------------------------

#[test]
fn add_task_inserts_after_specified_task() {
    let mut list = TaskList::new();
    let pid = list.add_phase("Build");
    let t1 = list.add_task(&pid, "Write code", TaskPosition::End).unwrap();
    let t2 = list.add_task(&pid, "Test code", TaskPosition::End).unwrap();
    // Insert t3 after t1: expected order [t1, t3, t2]
    let t3 = list
        .add_task(&pid, "Write docs", TaskPosition::After(t1.clone()))
        .unwrap();
    let phase = list.get_phase(&pid).unwrap();
    assert_eq!(phase.tasks().len(), 3);
    assert_eq!(&phase.tasks()[0].id, &t1);
    assert_eq!(&phase.tasks()[1].id, &t3);
    assert_eq!(&phase.tasks()[2].id, &t2);
}

// ---------------------------------------------------------------------------
// add_task — insert_before
// ---------------------------------------------------------------------------

#[test]
fn add_task_inserts_before_specified_task() {
    let mut list = TaskList::new();
    let pid = list.add_phase("Build");
    let t1 = list.add_task(&pid, "Write code", TaskPosition::End).unwrap();
    let t2 = list.add_task(&pid, "Test code", TaskPosition::End).unwrap();
    // Insert t3 before t2: expected order [t1, t3, t2]
    let t3 = list
        .add_task(&pid, "Write docs", TaskPosition::Before(t2.clone()))
        .unwrap();
    let phase = list.get_phase(&pid).unwrap();
    assert_eq!(phase.tasks().len(), 3);
    assert_eq!(&phase.tasks()[0].id, &t1);
    assert_eq!(&phase.tasks()[1].id, &t3);
    assert_eq!(&phase.tasks()[2].id, &t2);
}

// ---------------------------------------------------------------------------
// add_task — error cases
// ---------------------------------------------------------------------------

#[test]
fn add_task_rejects_invalid_phase_id() {
    let mut list = TaskList::new();
    let bad_id = PhaseId::new_for_test("p99");
    let result = list.add_task(&bad_id, "Task", TaskPosition::End);
    assert_eq!(result, Err(TaskListError::PhaseNotFound(bad_id)));
}

#[test]
fn add_task_with_after_rejects_task_not_in_phase() {
    let mut list = TaskList::new();
    let pid = list.add_phase("Phase A");
    let _t1 = list.add_task(&pid, "Task A1", TaskPosition::End).unwrap();
    let other_id = list.add_phase("Phase B");
    let other_t = list
        .add_task(&other_id, "Task B1", TaskPosition::End)
        .unwrap();

    // Try to insert after a task from a different phase.
    let result = list.add_task(&pid, "New", TaskPosition::After(other_t.clone()));
    assert_eq!(
        result,
        Err(TaskListError::TaskNotInPhase {
            task_id: other_t,
            phase_id: pid,
        })
    );
}

// ---------------------------------------------------------------------------
// complete_task
// ---------------------------------------------------------------------------

#[test]
fn complete_task_marks_as_completed() {
    let mut list = TaskList::new();
    let pid = list.add_phase("Build");
    let t1 = list.add_task(&pid, "Write code", TaskPosition::End).unwrap();
    let _t2 = list
        .add_task(&pid, "Write tests", TaskPosition::End)
        .unwrap();

    // Before: both pending.
    {
        let p = list.get_phase(&pid).unwrap();
        assert_eq!(p.tasks()[0].status(), TaskStatus::Pending);
        assert_eq!(p.tasks()[1].status(), TaskStatus::Pending);
    }

    list.complete_task(&t1).unwrap();

    // After: t1 completed, t2 still pending.
    {
        let p = list.get_phase(&pid).unwrap();
        assert_eq!(p.tasks()[0].status(), TaskStatus::Completed);
        assert_eq!(p.tasks()[1].status(), TaskStatus::Pending);
    }
}

#[test]
fn complete_task_rejects_unknown_id() {
    let mut list = TaskList::new();
    list.add_phase("Build");
    let bad_id = crate::feat::task_list::TaskId::new_for_test("t99");
    let result = list.complete_task(&bad_id);
    assert_eq!(result, Err(TaskListError::TaskNotFound(bad_id)));
}

// ---------------------------------------------------------------------------
// is_empty
// ---------------------------------------------------------------------------

#[test]
fn is_empty_true_when_no_phases() {
    let list = TaskList::new();
    assert!(list.is_empty());
}

#[test]
fn is_empty_false_when_has_phases() {
    let mut list = TaskList::new();
    list.add_phase("Build");
    assert!(!list.is_empty());
}

// ---------------------------------------------------------------------------
// get_phase / get_task
// ---------------------------------------------------------------------------

#[test]
fn get_phase_returns_none_for_missing_id() {
    let list = TaskList::new();
    let missing = PhaseId::new_for_test("p99");
    assert!(list.get_phase(&missing).is_none());
}

#[test]
fn get_task_finds_task_from_any_phase() {
    let mut list = TaskList::new();
    let p1 = list.add_phase("Research");
    let p2 = list.add_phase("Build");
    let t1 = list.add_task(&p1, "Read", TaskPosition::End).unwrap();
    let t2 = list.add_task(&p2, "Code", TaskPosition::End).unwrap();
    assert!(list.get_task(&t1).is_some());
    assert!(list.get_task(&t2).is_some());
}

#[test]
fn get_task_returns_none_for_missing_id() {
    let list = TaskList::new();
    let missing = crate::feat::task_list::TaskId::new_for_test("t99");
    assert!(list.get_task(&missing).is_none());
}

// ---------------------------------------------------------------------------
// render_text
// ---------------------------------------------------------------------------

#[test]
fn render_text_returns_empty_placeholder() {
    let list = TaskList::new();
    assert_eq!(list.render_text(), "No phases defined.");
}

#[test]
fn render_text_shows_phases_and_tasks() {
    let mut list = TaskList::new();
    let pid = list.add_phase("Research");
    let _t1 = list.add_task(&pid, "Read docs", TaskPosition::End).unwrap();
    let _t2 = list
        .add_task(&pid, "Call API", TaskPosition::End)
        .unwrap();

    let rendered = list.render_text();
    assert!(rendered.contains("Phase 1: Research"));
    assert!(rendered.contains("[ ] Read docs"));
    assert!(rendered.contains("[ ] Call API"));
    assert!(rendered.contains("[p1]"));
}

#[test]
fn render_text_shows_completed_task() {
    let mut list = TaskList::new();
    let pid = list.add_phase("Build");
    let t1 = list.add_task(&pid, "Write code", TaskPosition::End).unwrap();
    list.complete_task(&t1).unwrap();

    let rendered = list.render_text();
    assert!(rendered.contains("[✓] Write code"));
}

#[test]
fn render_phase_text_for_single_phase() {
    let mut list = TaskList::new();
    let pid = list.add_phase("Research");
    list.add_task(&pid, "Read docs", TaskPosition::End).unwrap();

    let rendered = list.render_phase_text(&pid).unwrap();
    assert!(rendered.contains("Phase 1: Research"));
    assert!(rendered.contains("Read docs"));
}

#[test]
fn render_phase_text_returns_none_for_missing_phase() {
    let list = TaskList::new();
    let missing = PhaseId::new_for_test("p99");
    assert!(list.render_phase_text(&missing).is_none());
}

// ---------------------------------------------------------------------------
// Serde roundtrip
// ---------------------------------------------------------------------------

#[test]
fn serde_roundtrip_preserves_state() {
    let mut list = TaskList::new();
    let pid = list.add_phase("Research");
    let t1 = list.add_task(&pid, "Read docs", TaskPosition::End).unwrap();
    list.complete_task(&t1).unwrap();
    list.add_task(&pid, "Call API", TaskPosition::End).unwrap();

    let json = serde_json::to_string(&list).unwrap();
    let restored: TaskList = serde_json::from_str(&json).unwrap();

    assert_eq!(list, restored);
    assert!(!restored.is_empty());

    let pid2 = list.add_phase("Research 2");
    let _t3 = list.add_task(&pid2, "More", TaskPosition::End).unwrap();
    let json2 = serde_json::to_string(&list).unwrap();
    let restored2: TaskList = serde_json::from_str(&json2).unwrap();
    assert_eq!(list, restored2);
}

#[test]
fn serde_default_creates_empty_list() {
    let json = "{}";
    let list: TaskList = serde_json::from_str(json).unwrap();
    assert!(list.is_empty());
}

#[test]
fn serde_deserializes_partial_json() {
    // Only phases field (no counters) — a valid old-format JSON.
    let json = r#"{"phases":[]}"#;
    let list: TaskList = serde_json::from_str(json).unwrap();
    assert!(list.is_empty());
}

// ---------------------------------------------------------------------------
// TaskId / PhaseId helpers (exposed for testing via pub(crate))
// ---------------------------------------------------------------------------

#[test]
fn id_display_format() {
    let pid = PhaseId::new_for_test("p1");
    let tid = crate::feat::task_list::TaskId::new_for_test("t2");
    assert_eq!(format!("{pid}"), "p1");
    assert_eq!(format!("{tid}"), "t2");
}
