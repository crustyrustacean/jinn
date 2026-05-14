//! System-ready actor — counts `ActorStarted` events and signals the main thread.
//!
//! Subscribes to [`ActorStarted`] and [`AllActorsSpawned`] events. Tracks the
//! running count of started actors. Only checks readiness after receiving
//! `AllActorsSpawned` (emitted by the wiring code after all actors are spawned).
//! When the received count matches the [`ActorCounter`] total, sends `()` on
//! a `oneshot::Sender` to unblock the main thread's `wait_for_system_ready` call.

use crate::common::actor::actor_counter::ActorCounter;
use crate::common::actor::protocol::event::{ActorStarted, AllActorsSpawned};
use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::protocol::Event;

/// The system-ready actor.
///
/// Waits for `AllActorsSpawned` (confirming all actors have been spawned),
/// then checks if the running count of `ActorStarted` events matches the
/// expected total from the injected `ActorCounter`. When both conditions are
/// met, sends `()` on the injected oneshot sender to wake the main thread.
pub struct SystemReadyActor {
    /// Oneshot sender to signal the main thread.
    ready_tx: Option<tokio::sync::oneshot::Sender<()>>,
    /// Number of `ActorStarted` events received so far.
    received: usize,
    /// Total number of actors in the system (shared counter).
    counter: ActorCounter,
    /// Whether `AllActorsSpawned` has been received.
    all_spawned: bool,
}

impl Actor for SystemReadyActor {
    type Message = NoDirectMsg;

    #[expect(
        clippy::expect_used,
        reason = "Data injection is required at activation"
    )]
    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<ActorStarted>();
        ctx.subscribe_event::<AllActorsSpawned>();
        ctx.set_description("Counts ActorStarted events and signals system ready");

        let ready_tx = ctx
            .take_data::<tokio::sync::oneshot::Sender<()>>()
            .expect("SystemReadyActor requires oneshot::Sender injection");
        let counter = ctx
            .take_data::<ActorCounter>()
            .expect("SystemReadyActor requires ActorCounter injection");

        Self {
            ready_tx: Some(ready_tx),
            received: 0,
            counter,
            all_spawned: false,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, _ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => {
                self.handle_event(&event);
            }
            ActorEnvelope::Command(_) | ActorEnvelope::System(_) => {}
        }
    }
}

impl SystemReadyActor {
    fn handle_event(&mut self, event: &Event) {
        match event {
            Event::ActorStarted(..) => {
                self.received += 1;
                self.maybe_signal_ready();
            }
            Event::AllActorsSpawned(..) => {
                self.all_spawned = true;
                self.maybe_signal_ready();
            }
            _ => {}
        }
    }

    /// Signals the main thread if all actors have been spawned AND all have started.
    fn maybe_signal_ready(&mut self) {
        if !self.all_spawned {
            return;
        }
        let expected = self.counter.load() as usize;
        if self.received >= expected {
            tracing::info!(received = self.received, expected, "actor system ready");
            if let Some(tx) = self.ready_tx.take() {
                let _ = tx.send(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::actor::{ActorContext, MessageSink, SendResult};
    use crate::protocol::Event;

    /// No-op sink for testing — records nothing.
    struct TestSink;
    impl MessageSink for TestSink {
        fn send_command(&self, _command: crate::protocol::Command) -> SendResult {
            Ok(())
        }
        fn send_event(&self, _event: crate::protocol::Event) -> SendResult {
            Ok(())
        }
    }

    #[expect(dead_code, reason = "test utility for future tests")]
    fn create_ctx() -> ActorContext {
        ActorContext::new("test", std::sync::Arc::new(TestSink))
    }

    #[rstest::rstest]
    fn does_not_signal_before_all_actors_spawned() {
        // Given a SystemReadyActor with counter=2 and 2 ActorStarted received.
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        let counter = ActorCounter::new();
        counter.increment();
        counter.increment();
        let mut actor = SystemReadyActor {
            ready_tx: Some(tx),
            received: 2,
            counter: counter.clone(),
            all_spawned: false,
        };

        // When processing another ActorStarted (count already matches).
        actor.handle_event(&Event::ActorStarted(
            crate::common::actor::protocol::event::ActorStarted {
                name: "test".to_owned(),
                description: None,
            },
        ));

        // Then the oneshot is NOT consumed (ready_tx still present).
        assert!(
            actor.ready_tx.is_some(),
            "should not signal before AllActorsSpawned"
        );
        drop(actor);
        assert!(rx.try_recv().is_err(), "oneshot should not be sent");
    }

    #[rstest::rstest]
    fn does_not_signal_when_count_does_not_match() {
        // Given a SystemReadyActor with counter=3, 2 ActorStarted, and AllActorsSpawned.
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        let counter = ActorCounter::new();
        counter.increment();
        counter.increment();
        counter.increment();
        let mut actor = SystemReadyActor {
            ready_tx: Some(tx),
            received: 2,
            counter: counter.clone(),
            all_spawned: false,
        };

        // When receiving AllActorsSpawned.
        actor.handle_event(&Event::AllActorsSpawned(
            crate::common::actor::protocol::event::AllActorsSpawned,
        ));

        // Then the oneshot is NOT consumed (count mismatch: 2 < 3).
        assert!(
            actor.ready_tx.is_some(),
            "should not signal when count mismatch"
        );
        drop(actor);
        assert!(rx.try_recv().is_err(), "oneshot should not be sent");
    }

    #[rstest::rstest]
    fn signals_when_all_spawned_and_count_matches() {
        // Given a SystemReadyActor with counter=2, 2 ActorStarted.
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        let counter = ActorCounter::new();
        counter.increment();
        counter.increment();
        let mut actor = SystemReadyActor {
            ready_tx: Some(tx),
            received: 2,
            counter: counter.clone(),
            all_spawned: false,
        };

        // When receiving AllActorsSpawned (count matches: 2 == 2).
        actor.handle_event(&Event::AllActorsSpawned(
            crate::common::actor::protocol::event::AllActorsSpawned,
        ));

        // Then the oneshot is consumed.
        assert!(actor.ready_tx.is_none(), "should have signaled");
        assert!(rx.try_recv().is_ok(), "oneshot should be sent");
    }

    #[rstest::rstest]
    fn signals_on_late_actor_started_after_all_spawned() {
        // Given a SystemReadyActor with counter=2, 1 ActorStarted, AllActorsSpawned received.
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        let counter = ActorCounter::new();
        counter.increment();
        counter.increment();
        let mut actor = SystemReadyActor {
            ready_tx: Some(tx),
            received: 1,
            counter: counter.clone(),
            all_spawned: false,
        };

        // First: receive AllActorsSpawned (count mismatch: 1 < 2).
        actor.handle_event(&Event::AllActorsSpawned(
            crate::common::actor::protocol::event::AllActorsSpawned,
        ));
        assert!(actor.ready_tx.is_some(), "should not signal yet");

        // Then: receive second ActorStarted (now 2 == 2).
        actor.handle_event(&Event::ActorStarted(
            crate::common::actor::protocol::event::ActorStarted {
                name: "late-actor".to_owned(),
                description: None,
            },
        ));

        // Then the oneshot is consumed.
        assert!(actor.ready_tx.is_none(), "should have signaled");
        assert!(rx.try_recv().is_ok(), "oneshot should be sent");
    }
}
