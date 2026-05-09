//! Central message router for commands and events.
//!
//! The [`Bus`] accepts handler registrations for specific message types, then
//! routes submitted commands and events to the matching handlers.
//!
//! # Processing model
//!
//! - [`process_commands`](Bus::process_commands) drains the command queue and
//!   dispatches each command to its registered handlers. If handlers submit new
//!   commands via [`Out`], those are processed in subsequent iterations
//!   (with a configurable [`max_iterations`](Bus::with_max_iterations) guard).
//! - [`process_events`](Bus::process_events) drains the event queue in a single
//!   pass. All handlers for each event always run (no interception).
//!
//! # Consistency
//!
//! Each command or event receives a fresh [`Out`] buffer. New messages
//! submitted by handlers are only queued after all handlers for the current item
//! have finished, ensuring a consistent state snapshot per dispatch.

/// A processed event ready for forwarding, with its source actor.
pub struct ProcessedEvent {
    /// The dispatched event.
    pub event: Event,
    /// The actor that originated this event, if any.
    pub source: Option<ActorName>,
}

/// A processed command ready for forwarding, with its source actor.
pub struct ProcessedCommand {
    /// The dispatched command.
    pub command: Command,
    /// The actor that originated this command, if any.
    pub source: Option<ActorName>,
}

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::marker::PhantomData;

use nullslop_protocol::provider::{RefreshModels, RescanPromptTemplates};
use nullslop_protocol::{ActorName, Command, CommandAction, Event};

use crate::handler::{CommandHandler, EventHandler, HandlerContext};
use crate::out::Out;

/// Type-erased command handler ready for dispatch.
struct AnyCommandHandler<S, Sv> {
    /// The type-erased handler instance.
    handler: Box<dyn Any>,
    /// Function pointer that downcasts and invokes the handler.
    invoke: fn(&dyn Any, &dyn Any, &mut S, &Sv, &mut Out) -> CommandAction,
    /// Marker for the unused services type parameter.
    _phantom: PhantomData<Sv>,
}

/// Type-erased event handler ready for dispatch.
struct AnyEventHandler<S, Sv> {
    /// The type-erased handler instance.
    handler: Box<dyn Any>,
    /// Function pointer that downcasts and invokes the handler.
    invoke: fn(&dyn Any, &dyn Any, &mut S, &Sv, &mut Out),
    /// Marker for the unused services type parameter.
    _phantom: PhantomData<Sv>,
}

/// Invokes a command handler with its concrete types.
#[expect(
    clippy::expect_used,
    reason = "type is guaranteed by construction via Bus registration"
)]
fn invoke_command<C, H, S, Sv>(
    handler: &dyn Any,
    cmd: &dyn Any,
    state: &mut S,
    services: &Sv,
    out: &mut Out,
) -> CommandAction
where
    H: CommandHandler<C, S, Sv> + 'static,
    C: 'static,
{
    let h = handler.downcast_ref::<H>().expect("handler type mismatch");
    let c = cmd.downcast_ref::<C>().expect("command type mismatch");
    let mut ctx = HandlerContext::new(state, services, out);
    h.handle(c, &mut ctx)
}

/// Invokes an event handler with its concrete types.
#[expect(
    clippy::expect_used,
    reason = "type is guaranteed by construction via Bus registration"
)]
fn invoke_event<E, H, S, Sv>(
    handler: &dyn Any,
    evt: &dyn Any,
    state: &mut S,
    services: &Sv,
    out: &mut Out,
) where
    H: EventHandler<E, S, Sv> + 'static,
    E: 'static,
{
    let h = handler.downcast_ref::<H>().expect("handler type mismatch");
    let e = evt.downcast_ref::<E>().expect("event type mismatch");
    let mut ctx = HandlerContext::new(state, services, out);
    h.handle(e, &mut ctx);
}

/// A queued command together with its origin.
struct QueuedCommand {
    /// The command payload.
    command: Command,
    /// The actor that submitted this command, if any.
    source: Option<ActorName>,
}

/// A queued event together with its origin.
struct QueuedEvent {
    /// The event payload.
    event: Event,
    /// The actor that submitted this event, if any.
    source: Option<ActorName>,
}

/// Central message router that dispatches commands and events to registered handlers.
///
/// Commands and events are submitted to queues and processed in order. Each
/// message is routed to every handler registered for its type. The processing
/// model ensures consistent state snapshots across handlers.
pub struct Bus<S, Sv> {
    /// Registered command handlers keyed by their command type.
    command_handlers: HashMap<TypeId, Vec<AnyCommandHandler<S, Sv>>>,
    /// Registered event handlers keyed by their event type.
    event_handlers: HashMap<TypeId, Vec<AnyEventHandler<S, Sv>>>,
    /// Commands waiting to be dispatched.
    command_queue: Vec<QueuedCommand>,
    /// Events waiting to be dispatched.
    event_queue: Vec<QueuedEvent>,
    /// Events dispatched during the last processing cycle, with source.
    /// Available via [`drain_processed_events`](Self::drain_processed_events).
    processed_events: Vec<ProcessedEvent>,
    /// Commands dispatched during the last processing cycle, with source.
    /// Available via [`drain_processed_commands`](Self::drain_processed_commands).
    processed_commands: Vec<ProcessedCommand>,
    /// Maximum number of processing iterations to prevent infinite loops.
    max_iterations: usize,
    /// Marker for the unused services type parameter.
    _phantom: PhantomData<Sv>,
}

impl<S, Sv> Bus<S, Sv> {
    /// Create a new bus with default settings.
    ///
    /// The default `max_iterations` is 100, which prevents infinite loops
    /// from misbehaving handlers that resubmit their own command type.
    #[must_use]
    pub fn new() -> Self {
        Self {
            command_handlers: HashMap::new(),
            event_handlers: HashMap::new(),
            command_queue: Vec::new(),
            event_queue: Vec::new(),
            processed_events: Vec::new(),
            processed_commands: Vec::new(),
            max_iterations: 100,
            _phantom: PhantomData,
        }
    }

    /// Set the maximum number of processing iterations for [`process_commands`](Self::process_commands).
    ///
    /// Prevents infinite loops when handlers resubmit commands during processing.
    /// The default is 100.
    #[must_use]
    pub fn with_max_iterations(mut self, max: usize) -> Self {
        self.max_iterations = max;
        self
    }

    /// Register a handler for a specific command type `C`.
    ///
    /// Multiple handlers can be registered for the same command type.
    /// They are called in registration order. The first handler to return
    /// [`CommandAction::Stop`] halts propagation.
    pub fn register_command_handler<C, H>(&mut self, handler: H)
    where
        C: 'static,
        H: CommandHandler<C, S, Sv> + 'static,
    {
        let type_id = TypeId::of::<C>();
        let invoke = invoke_command::<C, H, S, Sv>;
        let entry = AnyCommandHandler {
            handler: Box::new(handler),
            invoke,
            _phantom: PhantomData,
        };
        self.command_handlers
            .entry(type_id)
            .or_default()
            .push(entry);
    }

    /// Register a handler for a specific event type `E`.
    ///
    /// Multiple handlers can be registered for the same event type.
    /// All handlers always run — events have no interception.
    pub fn register_event_handler<E, H>(&mut self, handler: H)
    where
        E: 'static,
        H: EventHandler<E, S, Sv> + 'static,
    {
        let type_id = TypeId::of::<E>();
        let invoke = invoke_event::<E, H, S, Sv>;
        let entry = AnyEventHandler {
            handler: Box::new(handler),
            invoke,
            _phantom: PhantomData,
        };
        self.event_handlers.entry(type_id).or_default().push(entry);
    }

    /// Submit a command to the bus queue.
    ///
    /// The command will be dispatched when [`process_commands`](Self::process_commands) is called.
    /// The source is `None` (originated from the user or host, not an actor).
    pub fn submit_command(&mut self, cmd: Command) {
        self.submit_command_from(cmd, None);
    }

    /// Submit a command to the bus queue with an optional source actor name.
    ///
    /// The command will be dispatched when [`process_commands`](Self::process_commands) is called.
    pub fn submit_command_from(&mut self, cmd: Command, source: Option<ActorName>) {
        self.command_queue.push(QueuedCommand {
            command: cmd,
            source,
        });
    }

    /// Submit an event to the bus queue.
    ///
    /// The event will be dispatched when [`process_events`](Self::process_events) is called.
    /// The source is `None` (originated from the user or host, not an actor).
    pub fn submit_event(&mut self, evt: Event) {
        self.submit_event_from(evt, None);
    }

    /// Submit an event to the bus queue with an optional source actor name.
    ///
    /// The event will be dispatched when [`process_events`](Self::process_events) is called.
    pub fn submit_event_from(&mut self, evt: Event, source: Option<ActorName>) {
        self.event_queue.push(QueuedEvent { event: evt, source });
    }

    /// Process all pending commands, including those submitted by handlers.
    ///
    /// Drains the command queue, dispatches each command to its registered
    /// handlers, and repeats if handlers submitted new commands. Stops when
    /// the queue is empty or `max_iterations` is reached.
    pub fn process_commands(&mut self, state: &mut S, services: &Sv) {
        let mut iterations = 0;
        loop {
            let commands = std::mem::take(&mut self.command_queue);
            if commands.is_empty() {
                break;
            }
            iterations += 1;
            if iterations > self.max_iterations {
                break;
            }
            for queued in commands {
                self.dispatch_command(queued.command, queued.source, state, services);
            }
        }
    }

    /// Process all pending events in a single pass.
    ///
    /// Drains the event queue and dispatches each event to its registered
    /// handlers. All handlers always run. Events submitted by handlers during
    /// processing are queued for a future call.
    pub fn process_events(&mut self, state: &mut S, services: &Sv) {
        let events = std::mem::take(&mut self.event_queue);
        for queued in events {
            self.dispatch_event(queued.event, queued.source, state, services);
        }
    }

    /// Returns `true` if there are pending commands or events in the queues.
    #[must_use]
    pub fn has_pending(&self) -> bool {
        !self.command_queue.is_empty() || !self.event_queue.is_empty()
    }

    /// Returns `true` if there are pending commands in the queue.
    #[must_use]
    pub fn has_pending_commands(&self) -> bool {
        !self.command_queue.is_empty()
    }

    /// Drain all events that were dispatched during processing.
    ///
    /// Returns tuples of `(event, source)` and clears the internal buffer.
    /// Useful for forwarding processed events to external systems
    /// (e.g., actor host) after bus processing completes.
    pub fn drain_processed_events(&mut self) -> Vec<ProcessedEvent> {
        std::mem::take(&mut self.processed_events)
    }

    /// Drain all commands that were dispatched during processing.
    ///
    /// Returns tuples of `(command, source)` and clears the internal buffer.
    /// Useful for forwarding processed commands to external systems
    /// (e.g., actor host) after bus processing completes.
    pub fn drain_processed_commands(&mut self) -> Vec<ProcessedCommand> {
        std::mem::take(&mut self.processed_commands)
    }

    /// Drain all processed events and commands.
    ///
    /// Convenience method that returns both
    /// [`drain_processed_events`](Self::drain_processed_events) and
    /// [`drain_processed_commands`](Self::drain_processed_commands) as a tuple.
    pub fn drain_all(&mut self) -> (Vec<ProcessedEvent>, Vec<ProcessedCommand>) {
        let events = self.drain_processed_events();
        let commands = self.drain_processed_commands();
        (events, commands)
    }

    /// Dispatch a single command to its registered handlers.
    #[expect(
        clippy::too_many_lines,
        reason = "exhaustive match dispatch grows with each command variant"
    )]
    fn dispatch_command(
        &mut self,
        cmd: Command,
        source: Option<ActorName>,
        state: &mut S,
        services: &Sv,
    ) {
        // Record the command before dispatching so consumers can drain it later.
        self.processed_commands.push(ProcessedCommand {
            command: cmd.clone(),
            source,
        });
        let mut out = Out::new();
        match cmd {
            Command::SendMessage { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::SwitchPromptStrategy { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::RestoreStrategyState { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::PinChatEntry { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::UnpinChatEntry { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::EnqueueUserMessage { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::SetChatInputText { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::PushChatEntry { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::CancelStream { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::ProviderSwitch { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::AssemblePrompt { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::SendToLlmProvider { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::RefreshModels => {
                let cmd = RefreshModels;
                self.dispatch_command_to_handlers(&cmd, state, services, &mut out);
            }
            Command::RescanPromptTemplates => {
                let cmd = RescanPromptTemplates;
                self.dispatch_command_to_handlers(&cmd, state, services, &mut out);
            }
            Command::RegisterTools { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::ExecuteToolBatch { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::ExecuteTool { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::PushToolResult { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::ProceedWithShutdown { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::SessionLoadCompleted { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::LoadPickerEntries { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
            Command::SessionLoadRequested { payload } => {
                self.dispatch_command_to_handlers(&payload, state, services, &mut out);
            }
        }
        self.flush_out(out);
    }

    /// Look up and invoke handlers for a concrete command type `C`.
    fn dispatch_command_to_handlers<C>(&self, cmd: &C, state: &mut S, services: &Sv, out: &mut Out)
    where
        C: 'static,
    {
        let type_id = TypeId::of::<C>();
        if let Some(handlers) = self.command_handlers.get(&type_id) {
            for h in handlers {
                let action = (h.invoke)(&*h.handler, cmd as &dyn Any, state, services, out);
                if action == CommandAction::Stop {
                    break;
                }
            }
        }
    }

    /// Dispatch a single event to its registered handlers.
    fn dispatch_event(
        &mut self,
        evt: Event,
        source: Option<ActorName>,
        state: &mut S,
        services: &Sv,
    ) {
        // Record the event before dispatching so consumers can drain it later.
        self.processed_events.push(ProcessedEvent {
            event: evt.clone(),
            source,
        });
        let mut out = Out::new();
        match evt {
            Event::KeyDown { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::KeyUp { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::ChatEntrySubmitted { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::ModeChanged { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::ActorStarting { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::ActorStarted { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::ActorShutdownCompleted { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::StreamCompleted { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::StreamToken { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::ToolUseStarted { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::ToolCallReceived { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::ToolCallStreaming { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::ProviderSwitched { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::ModelsRefreshed { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::PromptTemplatesLoaded { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::ToolBatchCompleted { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::ToolExecutionCompleted { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::ToolsRegistered { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::PromptAssembled { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::PromptStrategySwitched { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::StrategyStateUpdated { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::SessionSaveRequested { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
            Event::SessionLoadRequested { payload } => {
                self.dispatch_event_to_handlers(&payload, state, services, &mut out);
            }
        }
        self.flush_out(out);
    }

    /// Look up and invoke handlers for a concrete event type `E`.
    fn dispatch_event_to_handlers<E>(&self, evt: &E, state: &mut S, services: &Sv, out: &mut Out)
    where
        E: 'static,
    {
        let type_id = TypeId::of::<E>();
        if let Some(handlers) = self.event_handlers.get(&type_id) {
            for h in handlers {
                (h.invoke)(&*h.handler, evt as &dyn Any, state, services, out);
            }
        }
    }

    /// Flush buffered output from a handler into the bus queues.
    fn flush_out(&mut self, mut out: Out) {
        for cmd in out.drain_commands() {
            self.command_queue.push(QueuedCommand {
                command: cmd,
                source: None,
            });
        }
        for evt in out.drain_events() {
            self.event_queue.push(QueuedEvent {
                event: evt,
                source: None,
            });
        }
    }
}

impl<S, Sv> Default for Bus<S, Sv> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod bus_tests;
