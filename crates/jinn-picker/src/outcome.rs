//! What a hook's run produces — messages, closure, and an optional scope transition.

use jinn_slices::PublishClosure;
use jinn_slices::PublishableMessage;
use jinn_slices::RouteResult;
use jinn_slices::ScopeSignal;

/// The outcome of a picker lifecycle hook or bind action.
///
/// Messages are erased publish closures in the exact shape the kernel's
/// drain task already consumes ([`PublishClosure`], minted through
/// [`RouteResult`]'s constructors so this crate stays publish-agnostic).
/// `close` requests that the picker and its overlays clear before the
/// optional `scope_signal` is applied.
#[derive(Default)]
pub struct PickerOutcome {
    /// Typed message closures to publish onto the fabric.
    pub messages: Vec<PublishClosure>,
    /// Type names of the messages, for test inspection.
    pub message_names: Vec<&'static str>,
    /// Whether the picker closes after this hook runs.
    pub close: bool,
    /// Scope transition to apply after optional picker closure.
    pub scope_signal: Option<ScopeSignal>,
}

impl std::fmt::Debug for PickerOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PickerOutcome")
            .field("messages", &self.messages.len())
            .field("message_names", &self.message_names)
            .field("close", &self.close)
            .field("scope_signal", &self.scope_signal)
            .finish()
    }
}

impl PickerOutcome {
    /// An outcome with no messages and no close.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// An outcome carrying one typed message, keeping the picker open.
    #[must_use]
    pub fn new_message<M>(msg: M) -> Self
    where
        M: PublishableMessage,
    {
        Self::from_route_result(RouteResult::new_message(msg))
    }

    /// Wraps a [`RouteResult`]'s messages and scope signal into a picker
    /// outcome while leaving the picker open.
    #[must_use]
    pub fn from_route_result(result: RouteResult) -> Self {
        Self {
            messages: result.messages,
            message_names: result.message_names,
            close: false,
            scope_signal: result.scope_signal,
        }
    }

    /// Appends a typed message, returning self for chaining.
    #[must_use]
    pub fn with_message<M: PublishableMessage>(mut self, msg: M) -> Self {
        let extra = Self::new_message(msg);
        self.messages.extend(extra.messages);
        self.message_names.extend(extra.message_names);
        self
    }

    /// Marks the picker to close before its optional scope transition.
    #[must_use]
    pub fn close(mut self) -> Self {
        self.close = true;
        self
    }

    /// Requests a scope transition after optional picker closure.
    #[must_use]
    pub fn with_scope_signal(mut self, signal: ScopeSignal) -> Self {
        self.scope_signal = Some(signal);
        self
    }

    /// Merges another outcome's messages and scope transition into this one.
    ///
    /// Messages append in receiver-then-argument order. Closure is requested
    /// if either outcome asks for it. The argument's scope signal replaces
    /// the receiver's signal when present; otherwise the receiver's signal
    /// remains.
    #[must_use]
    pub fn merge(mut self, other: Self) -> Self {
        self.messages.extend(other.messages);
        self.message_names.extend(other.message_names);
        self.close |= other.close;
        self.scope_signal = other.scope_signal.or(self.scope_signal);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jinn_slices::SliceScopeId;

    /// A schema'd stand-in message for closure-recording assertions.
    #[derive(Clone, serde::Serialize, serde::Deserialize, trouper::schema::Event)]
    #[schema(description = "Picker outcome closure test message.")]
    struct PickerOutcomeRecorded;

    #[rstest::rstest]
    #[test]
    fn new_message_records_the_message_type() {
        // Given a message outcome.
        let outcome = PickerOutcome::new_message(PickerOutcomeRecorded);

        // When inspecting the recorded names.
        // Then the message type name is recorded for test inspection.
        assert_eq!(outcome.message_names, ["PickerOutcomeRecorded"]);
        assert_eq!(outcome.messages.len(), 1);
        assert!(!outcome.close);
    }

    #[rstest::rstest]
    #[test]
    fn merge_combines_messages_and_close_wins() {
        // Given an open outcome and a closing outcome.
        let open = PickerOutcome::new_message(PickerOutcomeRecorded);
        let closing = PickerOutcome::empty().close();

        // When merging them.
        let merged = open.merge(closing);

        // Then the messages combine and the close wins.
        assert_eq!(merged.message_names.len(), 1);
        assert!(merged.close);
    }

    #[rstest::rstest]
    #[test]
    fn from_route_result_preserves_scope_signal() {
        // Given a route result requesting a destination scope.
        let destination = SliceScopeId::new("picker-test", "destination");
        let result = RouteResult::empty().with_scope_signal(ScopeSignal::Push(destination.clone()));

        // When wrapping it as a picker outcome.
        let outcome = PickerOutcome::from_route_result(result);

        // Then the destination is preserved and the picker remains open.
        assert_eq!(outcome.scope_signal, Some(ScopeSignal::Push(destination)));
        assert!(!outcome.close);
    }

    #[rstest::rstest]
    #[test]
    fn close_and_scope_signal_compose() {
        // Given a destination scope.
        let destination = SliceScopeId::new("picker-test", "destination");

        // When composing closure and the destination signal.
        let outcome = PickerOutcome::empty()
            .close()
            .with_scope_signal(ScopeSignal::Push(destination.clone()));

        // Then both transitions are retained.
        assert!(outcome.close);
        assert_eq!(outcome.scope_signal, Some(ScopeSignal::Push(destination)));
    }

    #[rstest::rstest]
    #[test]
    fn merge_preserves_receiver_scope_signal_when_argument_has_none() {
        // Given outcomes whose only signal is on the receiver.
        let first = SliceScopeId::new("picker-test", "first");
        let receiver = PickerOutcome::empty().with_scope_signal(ScopeSignal::Push(first.clone()));

        // When merging an outcome with no signal.
        let merged = receiver.merge(PickerOutcome::new_message(PickerOutcomeRecorded));

        // Then the receiver's signal remains.
        assert_eq!(merged.scope_signal, Some(ScopeSignal::Push(first)));
    }

    #[rstest::rstest]
    #[test]
    fn merge_argument_scope_signal_replaces_receiver_signal() {
        // Given receiver and argument signals.
        let first = SliceScopeId::new("picker-test", "first");
        let second = SliceScopeId::new("picker-test", "second");
        let receiver = PickerOutcome::empty().with_scope_signal(ScopeSignal::Push(first));
        let argument = PickerOutcome::empty().with_scope_signal(ScopeSignal::Push(second.clone()));

        // When merging the outcomes.
        let merged = receiver.merge(argument);

        // Then the later signal wins.
        assert_eq!(merged.scope_signal, Some(ScopeSignal::Push(second)));
    }
}
