//! Closure-based routing for heterogeneous actor message dispatch.
//!
//! [`RoutingEntry`] wraps a typed actor sender in closures, enabling the host
//! to route events and commands to actors with different message types without
//! generics on the host itself.

use crate::protocol::{Command, CommandName, Event, EventTypeName};

/// A routing entry that wraps a typed actor sender in closures.
///
/// Created during [`spawn_actor_impl`](crate::spawn_actor_impl) by capturing a cloned
/// [`ActorRef<M>`](crate::common::actor::ActorRef). Stored in
/// `HashMap<String, Vec<RoutingEntry>>` — no type parameter, enabling
/// heterogeneous collections of actors with different message types.
pub struct RoutingEntry {
    /// The actor's unique name (for source filtering).
    pub name: String,
    /// Event type names this actor subscribed to during activation.
    pub subscriptions: Vec<EventTypeName>,
    /// Command names this actor registered for during activation.
    pub commands: Vec<CommandName>,
    /// Sends an event to this actor (wraps in `ActorEnvelope::Event`).
    pub send_event: Box<dyn Fn(Event) + Send + Sync>,
    /// Sends a command to this actor (wraps in `ActorEnvelope::Command`).
    pub send_command: Box<dyn Fn(Command) + Send + Sync>,
    /// Sends a system message to this actor (wraps in `ActorEnvelope::System`).
    pub send_system: Box<dyn Fn(crate::common::actor::SystemMessage) + Send + Sync>,
    /// Closes the actor's channel, causing its run loop to exit.
    pub close_channel: Box<dyn Fn() + Send + Sync>,
}

#[cfg(test)]
mod tests {
    use crate::common::actor::{ActorEnvelope, ActorRef};
    use kanal::Receiver;

    fn make_actor_ref_and_rx() -> (ActorRef<String>, Receiver<ActorEnvelope<String>>) {
        let (tx, rx) = kanal::unbounded::<ActorEnvelope<String>>();
        (ActorRef::new(tx), rx)
    }

    #[rstest::rstest]
    fn send_event_closure_wraps_and_delivers() {
        // Given a RoutingEntry built from an ActorRef<String>.
        let (actor_ref, rx) = make_actor_ref_and_rx();
        let ref_clone = actor_ref.clone();
        let entry = super::RoutingEntry {
            name: "test".to_owned(),
            subscriptions: vec![],
            commands: vec![],
            send_event: Box::new(move |event| {
                let _ = ref_clone.send_event(event);
            }),
            send_command: Box::new(|_| {}),
            send_system: Box::new(|_| {}),
            close_channel: Box::new(|| {}),
        };

        // When calling send_event with a ModeChanged event.
        (entry.send_event)(crate::protocol::Event::ModeChanged {
            payload: crate::protocol::system::ModeChanged {
                from: crate::protocol::Mode::Normal,
                to: crate::protocol::Mode::Input,
            },
        });

        // Then it is received as an Event envelope.
        let msg = rx
            .try_recv()
            .expect("recv should succeed")
            .expect("should have value");
        assert!(matches!(
            msg,
            ActorEnvelope::Event(crate::protocol::Event::ModeChanged { .. })
        ));
    }

    #[rstest::rstest]
    fn send_command_closure_wraps_and_delivers() {
        // Given a RoutingEntry built from an ActorRef<String>.
        let (actor_ref, rx) = make_actor_ref_and_rx();
        let ref_clone = actor_ref.clone();
        let entry = super::RoutingEntry {
            name: "test".to_owned(),
            subscriptions: vec![],
            commands: vec![],
            send_event: Box::new(|_| {}),
            send_command: Box::new(move |command| {
                let _ = ref_clone.send_command(command);
            }),
            send_system: Box::new(|_| {}),
            close_channel: Box::new(|| {}),
        };

        // When calling send_command.
        (entry.send_command)(crate::protocol::Command::RefreshModels);

        // Then it is received as a Command envelope.
        let msg = rx
            .try_recv()
            .expect("recv should succeed")
            .expect("should have value");
        assert!(matches!(
            msg,
            ActorEnvelope::Command(crate::protocol::Command::RefreshModels)
        ));
    }

    #[rstest::rstest]
    fn send_system_closure_wraps_and_delivers() {
        // Given a RoutingEntry built from an ActorRef<String>.
        let (actor_ref, rx) = make_actor_ref_and_rx();
        let ref_clone = actor_ref.clone();
        let entry = super::RoutingEntry {
            name: "test".to_owned(),
            subscriptions: vec![],
            commands: vec![],
            send_event: Box::new(|_| {}),
            send_command: Box::new(|_| {}),
            send_system: Box::new(move |msg| {
                let _ = ref_clone.send_system(msg);
            }),
            close_channel: Box::new(|| {}),
        };

        // When calling send_system with ApplicationShuttingDown.
        (entry.send_system)(crate::common::actor::SystemMessage::ApplicationShuttingDown);

        // Then it is received as a System envelope.
        let msg = rx
            .try_recv()
            .expect("recv should succeed")
            .expect("should have value");
        assert!(matches!(
            msg,
            ActorEnvelope::System(crate::common::actor::SystemMessage::ApplicationShuttingDown)
        ));
    }
}
