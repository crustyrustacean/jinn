//! Tests for echo message rendering.
//!
//! Observable behavior: the synthetic user message content (wrapper contract,
//! tree-only, cap, determinism) and the `None` skip for empty lists.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use jinn_provider::LlmMessage;

use super::render::{ECHO_HEADER, ECHO_PREAMBLE, echo_message};
use crate::feat::todo_list::TaskList;

/// Builds a two-phase list with two tasks in the first phase, one in the
/// second. Same shape as the todo tool flows produce.
fn populated_task_list() -> TaskList {
    let mut list = TaskList::new();
    let p1 = list.add_phase("Research");
    list.add_task(&p1, "First task", crate::feat::todo_list::TaskPosition::End)
        .expect("add first task");
    list.add_task(
        &p1,
        "Second task",
        crate::feat::todo_list::TaskPosition::End,
    )
    .expect("add second task");
    let p2 = list.add_phase("Build");
    list.add_task(&p2, "Third task", crate::feat::todo_list::TaskPosition::End)
        .expect("add third task");
    list
}

/// Builds a list rendering exactly `n` lines.
fn task_list_with_n_render_lines(n: usize) -> TaskList {
    let mut list = TaskList::new();
    let p = list.add_phase("Filler");
    // Each task renders as one line; two lines are the phase header and the
    // joined non-empty phases, so seed enough tasks to exceed any target.
    for i in 0..n {
        list.add_task(
            &p,
            &format!("Task {i}"),
            crate::feat::todo_list::TaskPosition::End,
        )
        .expect("add task");
    }
    list
}

#[rstest::rstest]
fn echo_renders_wrapper_and_tree_without_next_line() {
    // Given a populated task list.
    let list = populated_task_list();

    // When building the echo.
    let msg = echo_message(&list, 60).expect("echo for non-empty list");

    // Then the message is a user message containing the header.
    let LlmMessage::User {
        content,
        attachments,
    } = &msg
    else {
        panic!("expected user message, got {msg:?}");
    };
    assert_eq!(content.split("\n\n").next(), Some(ECHO_HEADER));
    // And it contains the treatment contract.
    assert!(
        content.contains(ECHO_PREAMBLE),
        "preamble missing: {content}"
    );
    // And it contains the tree.
    assert!(
        content.contains("## Phase 1: Research"),
        "tree missing: {content}"
    );
    assert!(content.contains("First task"), "task missing: {content}");
    // And it does not contain the next-step line.
    assert!(
        !content.contains("→ NEXT"),
        "echo must be tree-only, got: {content}"
    );
    // And it carries no attachments.
    assert!(attachments.is_empty());
}

#[rstest::rstest]
fn echo_tree_capped_with_overflow_suffix() {
    // Given a list whose tree renders more lines than the cap.
    let list = task_list_with_n_render_lines(8);
    let tree_lines = list.render_text_with_blockers().lines().count();
    assert!(tree_lines > 3, "fixture must overflow: {tree_lines}");
    let cap = 3;

    // When building the echo with the small cap.
    let msg = echo_message(&list, cap).expect("echo for non-empty list");

    // Then the content contains exactly `cap` tree lines plus the suffix.
    let LlmMessage::User { content, .. } = msg else {
        panic!("expected user message");
    };
    let rendered_tree = content
        .split_once(ECHO_PREAMBLE)
        .expect("preamble present")
        .1
        .trim_start_matches('\n');
    let lines: Vec<&str> = rendered_tree.lines().collect();
    assert_eq!(lines.len(), cap + 1, "cap plus one suffix line: {lines:?}");
    // And the suffix names the dropped count and the tool to re-sync with.
    assert_eq!(
        lines[cap],
        format!("… {} more lines — see get_task_list", tree_lines - cap),
    );
}

#[rstest::rstest]
fn echo_skipped_when_task_list_empty() {
    // Given an empty task list.
    let list = TaskList::new();

    // When building the echo.
    let msg = echo_message(&list, 60);

    // Then no message is produced.
    assert!(msg.is_none());
}

#[rstest::rstest]
fn echo_output_is_deterministic() {
    // Given a populated task list rendered twice (as consecutive assemblies
    // of an unchanged list would render it).
    let list = populated_task_list();

    // When building echoes from the same list twice.
    let first = echo_message(&list, 60).expect("echo first");
    let second = echo_message(&list, 60).expect("echo second");

    // Then the outputs are byte-identical.
    assert_eq!(first, second);
}
