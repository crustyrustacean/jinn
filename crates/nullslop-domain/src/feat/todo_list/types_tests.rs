//! Tests for the task list data model.
//!
//! BDD-style tests following AGENTS.md conventions.
//! Each test covers a single behavior.

use crate::feat::todo_list::{PhaseId, TaskId, TaskList, TaskListError, TaskPosition, TaskStatus};

// ---------------------------------------------------------------------------
// add_phase
// ---------------------------------------------------------------------------

#[test]
fn add_phase_creates_phase_with_id_and_description() {
    let mut list = TaskList::new();
    let id = list.add_phase("Research");
    // ID should start with 'p' and be 4 chars total (prefix + 3 random chars).
    let id_str = id.to_string();
    assert!(id_str.starts_with('p'));
    assert_eq!(id_str.len(), 4);
    let phase = list.get_phase(&id).unwrap();
    assert_eq!(phase.description(), "Research");
    assert!(phase.is_empty());
}

#[test]
fn add_phase_generates_distinct_ids() {
    let mut list = TaskList::new();
    let id1 = list.add_phase("Research");
    let id2 = list.add_phase("Build");
    let id3 = list.add_phase("Test");
    assert_ne!(id1, id2);
    assert_ne!(id2, id3);
    assert_ne!(id1, id3);
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
    let bad_id = crate::feat::todo_list::TaskId::new_for_test("t99");
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
    let missing = crate::feat::todo_list::TaskId::new_for_test("t99");
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
    // Phase ID should appear in the output.
    assert!(rendered.contains(&format!("[{pid}]")));
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
    let tid = crate::feat::todo_list::TaskId::new_for_test("t2");
    assert_eq!(format!("{pid}"), "p1");
    assert_eq!(format!("{tid}"), "t2");
}

// ---------------------------------------------------------------------------
// Random ID generation
// ---------------------------------------------------------------------------

#[test]
fn id_format_is_correct() {
    // Phase IDs start with 'p' and are 4 chars total.
    // Task IDs start with 't' and are 4 chars total.
    // The remaining 3 chars are from the charset [a-z0-9 minus {p, t}].
    let mut list = TaskList::new();
    let pid = list.add_phase("Phase");
    let tid = list.add_task(&pid, "Task", TaskPosition::End).unwrap();

    let pid_str = pid.to_string();
    let tid_str = tid.to_string();

    assert_eq!(pid_str.len(), 4);
    assert!(pid_str.starts_with('p'));
    // The 3 random chars should not contain 'p' or 't'.
    let suffix = &pid_str[1..];
    for ch in suffix.chars() {
        assert!(ch.is_ascii_alphanumeric());
        assert!(ch != 'p' && ch != 't', "char '{ch}' should not be 'p' or 't'");
    }

    assert_eq!(tid_str.len(), 4);
    assert!(tid_str.starts_with('t'));
    let suffix = &tid_str[1..];
    for ch in suffix.chars() {
        assert!(ch.is_ascii_alphanumeric());
        assert!(ch != 'p' && ch != 't', "char '{ch}' should not be 'p' or 't'");
    }
}

#[test]
fn id_generation_no_collision() {
    let mut list = TaskList::new();
    let mut phase_ids = Vec::new();
    let mut task_ids = Vec::new();

    for i in 0..50 {
        let pid = list.add_phase(&format!("Phase {i}"));
        phase_ids.push(pid.clone());
        let tid = list.add_task(&pid, &format!("Task {i}"), TaskPosition::End).unwrap();
        task_ids.push(tid);
    }

    // All phase IDs are unique.
    let mut sorted_pids = phase_ids.clone();
    sorted_pids.sort();
    sorted_pids.dedup();
    assert_eq!(sorted_pids.len(), phase_ids.len());

    // All task IDs are unique.
    let mut unique_tids = task_ids.clone();
    unique_tids.sort();
    unique_tids.dedup();
    assert_eq!(unique_tids.len(), task_ids.len());
}

#[test]
fn serde_backward_compat_with_counters() {
    // Old-format JSON with counter fields should deserialize cleanly.
    let json = r#"{"phases":[],"next_phase_id":5,"next_task_id":10}"#;
    let mut list: TaskList = serde_json::from_str(json).unwrap();
    assert!(list.is_empty());
    // Should be able to add phases/tasks after loading old format.
    let pid = list.add_phase("New phase");
    let tid = list.add_task(&pid, "New task", TaskPosition::End).unwrap();
    assert_eq!(pid.to_string().len(), 4);
    assert_eq!(tid.to_string().len(), 4);
}

#[test]
fn defer_task_marks_source_and_creates_copy() {
    let mut list = TaskList::new();
    let p1 = list.add_phase("Research");
    let t1 = list.add_task(&p1, "Read docs", TaskPosition::End).unwrap();
    let p2 = list.add_phase("Build");
    let t2 = list.add_task(&p2, "Write code", TaskPosition::End).unwrap();

    let new_tid = list.defer_task(&t1, TaskPosition::After(t2.clone())).unwrap();

    // Source task should be deferred.
    let source = list.get_task(&t1).unwrap();
    assert_eq!(source.status(), TaskStatus::Deferred);

    // New task should be pending in phase 2.
    let copy = list.get_task(&new_tid).unwrap();
    assert_eq!(copy.status(), TaskStatus::Pending);
    assert_eq!(copy.description(), "Read docs");
    assert_ne!(copy.id(), source.id());

    // Phase 2 should have the original task + the copy.
    let phase2 = list.get_phase(&p2).unwrap();
    assert_eq!(phase2.tasks().len(), 2);
}

#[test]
fn defer_task_same_phase() {
    let mut list = TaskList::new();
    let p1 = list.add_phase("Research");
    let t1 = list.add_task(&p1, "Read docs", TaskPosition::End).unwrap();
    let t2 = list.add_task(&p1, "Call API", TaskPosition::End).unwrap();

    let new_tid = list.defer_task(&t1, TaskPosition::Before(t2.clone())).unwrap();

    // Source deferred.
    let source = list.get_task(&t1).unwrap();
    assert_eq!(source.status(), TaskStatus::Deferred);

    // Copy is pending with same description.
    let copy = list.get_task(&new_tid).unwrap();
    assert_eq!(copy.status(), TaskStatus::Pending);
    assert_eq!(copy.description(), "Read docs");

    // Same phase has 3 tasks now (deferred + pending copy + original pending).
    let phase = list.get_phase(&p1).unwrap();
    assert_eq!(phase.tasks().len(), 3);
}

#[test]
fn defer_task_error_on_missing_source() {
    let mut list = TaskList::new();
    let p1 = list.add_phase("Build");
    let t1 = list.add_task(&p1, "Write code", TaskPosition::End).unwrap();

    let fake_id = TaskId::new_for_test("t99");
    let result = list.defer_task(&fake_id, TaskPosition::After(t1));
    assert!(matches!(result, Err(TaskListError::TaskNotFound(_))));
}

#[test]
fn defer_task_error_on_missing_reference() {
    let mut list = TaskList::new();
    let p1 = list.add_phase("Build");
    let t1 = list.add_task(&p1, "Write code", TaskPosition::End).unwrap();

    let fake_ref = TaskId::new_for_test("t99");
    let result = list.defer_task(&t1, TaskPosition::After(fake_ref));
    assert!(matches!(result, Err(TaskListError::TaskNotFound(_))));
}

#[test]
fn defer_task_error_on_self_reference() {
    let mut list = TaskList::new();
    let p1 = list.add_phase("Build");
    let t1 = list.add_task(&p1, "Write code", TaskPosition::End).unwrap();

    let result = list.defer_task(&t1, TaskPosition::After(t1.clone()));
    assert!(matches!(result, Err(TaskListError::SelfReference(_))));
}

#[test]
fn defer_task_error_on_already_deferred() {
    let mut list = TaskList::new();
    let p1 = list.add_phase("Research");
    let t1 = list.add_task(&p1, "Read docs", TaskPosition::End).unwrap();
    let p2 = list.add_phase("Build");
    let t2 = list.add_task(&p2, "Write code", TaskPosition::End).unwrap();

    // Defer once.
    list.defer_task(&t1, TaskPosition::After(t2.clone())).unwrap();

    // Try to defer again.
    let result = list.defer_task(&t1, TaskPosition::After(t2));
    assert!(matches!(result, Err(TaskListError::AlreadyDeferred(_))));
}

#[test]
fn render_text_excludes_deferred() {
    let mut list = TaskList::new();
    let p1 = list.add_phase("Research");
    let t1 = list.add_task(&p1, "Read docs", TaskPosition::End).unwrap();
    let p2 = list.add_phase("Build");
    let t2 = list.add_task(&p2, "Write code", TaskPosition::End).unwrap();

    let new_tid = list.defer_task(&t1, TaskPosition::After(t2.clone())).unwrap();

    let rendered = list.render_text();

    // The deferred source should NOT appear as a task line.
    assert!(
        !rendered.contains(&format!("- [ ] Read docs [{t1}]")),
        "deferred task should not appear in render"
    );
    // The copy should appear.
    assert!(
        rendered.contains(&format!("- [ ] Read docs [{new_tid}]")),
        "copy should appear in render"
    );
}

#[test]
fn render_text_shows_no_tasks_when_all_deferred() {
    let mut list = TaskList::new();
    let p1 = list.add_phase("Research");
    let t1 = list.add_task(&p1, "Read docs", TaskPosition::End).unwrap();
    let p2 = list.add_phase("Build");
    let t2 = list.add_task(&p2, "Write code", TaskPosition::End).unwrap();

    // Defer the only task in phase 1.
    list.defer_task(&t1, TaskPosition::After(t2)).unwrap();

    let rendered = list.render_text();

    // Phase 1 should show (no tasks) since its only task is deferred.
    let phase1_section = rendered.split("Phase 1").nth(1).unwrap();
    let phase1_text = phase1_section.split("Phase 2").next().unwrap();
    assert!(
        phase1_text.contains("(no tasks)"),
        "phase with all deferred tasks should show (no tasks)"
    );
}

#[test]
fn render_phase_text_excludes_deferred() {
    let mut list = TaskList::new();
    let p1 = list.add_phase("Research");
    let t1 = list.add_task(&p1, "Read docs", TaskPosition::End).unwrap();
    let p2 = list.add_phase("Build");
    let t2 = list.add_task(&p2, "Write code", TaskPosition::End).unwrap();

    list.defer_task(&t1, TaskPosition::After(t2)).unwrap();

    let rendered = list.render_phase_text(&p1).unwrap();
    assert!(
        rendered.contains("(no tasks)"),
        "phase with only deferred task should show (no tasks)"
    );
}
