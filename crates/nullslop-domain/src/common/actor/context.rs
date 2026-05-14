//! Actor context for subscriptions, peer references, and sending messages.
//!
//! [`ActorContext`] is provided to actor methods. During [`activate`](crate::Actor::activate),
//! the context accumulates subscriptions and provides peer [`ActorRef<M>`](crate::ActorRef)
//! handles via `take_actor_ref`. During `handle`,
//! the context can send commands and events back to the application via the
//! [`MessageSink`] trait.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use crate::common::actor::protocol::event::{ActorShutdownCompleted, ActorStarted};
use crate::protocol::{Command, CommandMsg, CommandName, Event, EventMsg, EventTypeName};

use super::ActorRef;
use super::actor_ref::SendResult;
use super::message_sink::MessageSink;

/// Context provided to actor methods.
///
/// During [`activate`](crate::Actor::activate), the context accumulates subscriptions
/// and provides peer [`ActorRef<M>`](crate::ActorRef) handles via
/// `take_actor_ref`. During `handle`, the context
/// can send commands and events back to the application via the
/// [`MessageSink`] trait.
pub struct ActorContext {
    /// The actor's host-assigned name.
    name: String,
    /// A short human-readable description of what the actor does.
    description: Option<String>,
    /// Accumulated event subscriptions (by type name).
    subscriptions: Vec<EventTypeName>,
    /// Accumulated command registrations (by name).
    commands: Vec<CommandName>,
    /// Whether this actor subscribes to ALL events (wildcard).
    subscribes_all_events: bool,
    /// Type-keyed actor ref storage, keyed by `TypeId::of::<M>()`.
    actor_refs: HashMap<TypeId, Box<dyn Any + Send + Sync>>, // Actually Box<ActorRef<M>>
    /// Message sink for sending commands/events to the application.
    sink: Arc<dyn MessageSink>,
    /// Type-erased data storage for constructor injection.
    ///
    /// Uses the same `TypeId` pattern as `actor_refs`.
    data: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl ActorContext {
    /// Creates a new actor context with the given name and message sink.
    ///
    /// Called by the actor host during startup — actor authors typically
    /// don't construct this directly.
    #[must_use]
    pub fn new<S>(name: S, sink: Arc<dyn MessageSink>) -> Self
    where
        S: AsRef<str>,
    {
        Self {
            name: name.as_ref().to_owned(),
            description: None,
            subscriptions: Vec::new(),
            commands: Vec::new(),
            subscribes_all_events: false,
            actor_refs: HashMap::new(),
            sink,
            data: HashMap::new(),
        }
    }

    /// Returns the actor's host-assigned name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Sets a short human-readable description for this actor.
    ///
    /// Called during [`activate`](crate::Actor::activate). The description
    /// is included in lifecycle events for display on the dashboard.
    pub fn set_description<S>(&mut self, description: S)
    where
        S: AsRef<str>,
    {
        self.description = Some(description.as_ref().to_owned());
    }

    /// Returns the actor's description, if set.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Subscribes to a bus event by name.
    ///
    /// For compile-time-checked subscriptions, prefer
    /// [`subscribe_event`](Self::subscribe_event).
    pub fn subscribe_event_by_name<N>(&mut self, name: N)
    where
        N: Into<EventTypeName>,
    {
        self.subscriptions.push(name.into());
    }

    /// Subscribes to a typed bus event.
    ///
    /// Uses the [`EventMsg::TYPE_NAME`] constant for routing,
    /// providing compile-time validation.
    pub fn subscribe_event<T>(&mut self)
    where
        T: EventMsg,
    {
        self.subscriptions.push(T::TYPE_NAME.to_owned());
    }

    /// Subscribes to a bus command by name.
    ///
    /// For compile-time-checked subscriptions, prefer
    /// [`subscribe_command`](Self::subscribe_command).
    pub fn subscribe_command_by_name<N>(&mut self, name: N)
    where
        N: Into<CommandName>,
    {
        self.commands.push(name.into());
    }

    /// Subscribes to a typed bus command.
    ///
    /// Uses the [`CommandMsg::NAME`] constant for routing,
    /// providing compile-time validation.
    pub fn subscribe_command<T>(&mut self)
    where
        T: CommandMsg,
    {
        self.commands.push(T::NAME.to_owned());
    }

    /// Stores an [`ActorRef<M>`] keyed by the message type `M`.
    ///
    /// The actor retrieves it during activation with
    /// [`take_actor_ref::<M>()`](Self::take_actor_ref).
    pub fn set_actor_ref<M>(&mut self, actor_ref: ActorRef<M>)
    where
        M: Send + 'static,
    {
        self.actor_refs
            .insert(TypeId::of::<M>(), Box::new(actor_ref));
    }

    /// Removes and returns the [`ActorRef<M>`] for message type `M`.
    ///
    /// Returns `None` if no `ActorRef` was stored for this message type.
    /// This is a take (not a clone) — subsequent calls return `None`.
    pub fn take_actor_ref<M>(&mut self) -> Option<ActorRef<M>>
    where
        M: Send + 'static,
    {
        self.actor_refs
            .remove(&TypeId::of::<M>())
            .and_then(|boxed| boxed.downcast::<ActorRef<M>>().ok())
            .map(|boxed| *boxed)
    }

    /// Stores typed data for constructor injection.
    ///
    /// Uses the same `TypeId` pattern as [`set_actor_ref`](Self::set_actor_ref).
    /// The wiring code injects data before calling `activate`, and `activate`
    /// extracts it via [`take_data`](Self::take_data).
    pub fn set_data<T>(&mut self, data: T)
    where
        T: Send + Sync + 'static,
    {
        self.data.insert(TypeId::of::<T>(), Box::new(data));
    }

    /// Removes and returns injected data of type `T`.
    ///
    /// Returns `None` if no data of this type was stored.
    /// This is a take (not a clone) — subsequent calls return `None`.
    pub fn take_data<T>(&mut self) -> Option<T>
    where
        T: Send + Sync + 'static,
    {
        self.data
            .remove(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast::<T>().ok())
            .map(|boxed| *boxed)
    }

    /// Sends a command to the application via the message sink.
    ///
    /// # Errors
    ///
    /// Returns an error if the message sink fails to deliver.
    pub fn send_command(&self, command: Command) -> SendResult {
        self.sink.send_command(command)
    }

    /// Sends an event to the application via the message sink.
    ///
    /// # Errors
    ///
    /// Returns an error if the message sink fails to deliver.
    pub fn send_event(&self, event: Event) -> SendResult {
        self.sink.send_event(event)
    }

    /// Returns a reference-counted clone of the message sink.
    ///
    /// Useful for passing the sink to spawned tasks that need to
    /// send commands or events independently of the actor context.
    #[must_use]
    pub fn sink(&self) -> Arc<dyn MessageSink> {
        self.sink.clone()
    }

    /// Marks this actor as subscribing to ALL events (wildcard).
    ///
    /// Actors that call this will receive every event broadcast on the bus,
    /// regardless of individual event type subscriptions.
    pub fn subscribe_all_events(&mut self) {
        self.subscribes_all_events = true;
    }

    /// Returns the accumulated event subscriptions and command registrations,
    /// clearing them from the context.
    ///
    /// Returns `(event_subscriptions, command_registrations, subscribes_all_events)`.
    /// The host calls this after activation to set up bus routing.
    pub fn take_registrations(&mut self) -> (Vec<EventTypeName>, Vec<CommandName>, bool) {
        let subscriptions = std::mem::take(&mut self.subscriptions);
        let commands = std::mem::take(&mut self.commands);
        let all = self.subscribes_all_events;
        (subscriptions, commands, all)
    }

    /// Announces that this actor has finished starting up.
    ///
    /// Sends `Event::ActorStarted` with the actor's name. Fire-and-forget —
    /// logs a warning on send failure but does not propagate the error.
    pub fn announce_started(&self) {
        if let Err(e) = self.send_event(Event::ActorStarted(ActorStarted {
            name: self.name.clone(),
            description: self.description.clone(),
        })) {
            tracing::warn!(name = %self.name, err = ?e, "failed to announce ActorStarted");
        }
    }

    /// Announces that this actor has completed shutdown.
    ///
    /// Sends `Event::ActorShutdownCompleted` with the actor's name. Fire-and-forget —
    /// logs a warning on send failure but does not propagate the error.
    pub fn announce_shutdown_completed(&self) {
        if let Err(e) = self.send_event(Event::ActorShutdownCompleted(ActorShutdownCompleted {
            name: self.name.clone(),
        })) {
            tracing::warn!(name = %self.name, err = ?e, "failed to announce ActorShutdownCompleted");
        }
    }
}
