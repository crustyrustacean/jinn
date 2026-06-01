//! Workflow response — the action contract between workflow nodes and the controller.
//!
//! [`WorkflowResponse`] is the composable action unit returned by workflow terminal nodes.
//! The controller iterates `Vec<WorkflowResponse>` and applies each action in order.

use crate::protocol::ChatEntry;

/// An action returned by a workflow terminal node.
///
/// Controller iterates `Vec<WorkflowResponse>` and applies each in order.
/// Composable: judge approval is `vec![PushSessionHistory(system), TurnOff]`,
/// consensus one-shot is `vec![PushSessionHistory(assistant), Detach]`.
#[derive(Debug, Clone)]
pub enum WorkflowResponse {
    /// Set `enabled = false` on the attachment (soft disable).
    TurnOff,
    /// Remove the attachment from the session entirely.
    Detach,
    /// Push this entry into the session history.
    PushSessionHistory(ChatEntry),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ChatEntryKind;

    // --- Test: workflow_response_enum_covers_all_variants ---

    #[rstest::rstest]
    fn workflow_response_enum_covers_all_variants() {
        // Verify all variants compile and can be constructed.
        let turn_off = WorkflowResponse::TurnOff;
        let detach = WorkflowResponse::Detach;
        let push = WorkflowResponse::PushSessionHistory(ChatEntry::assistant("test"));

        // Exhaustive match ensures no variant is missed.
        for response in [turn_off, detach, push] {
            match response {
                WorkflowResponse::TurnOff => {}
                WorkflowResponse::Detach => {}
                WorkflowResponse::PushSessionHistory(ref entry) => {
                    if let ChatEntryKind::Assistant(text) = &entry.kind {
                        assert_eq!(text, "test");
                    } else {
                        panic!("expected Assistant entry");
                    }
                }
            }
        }
    }
}
