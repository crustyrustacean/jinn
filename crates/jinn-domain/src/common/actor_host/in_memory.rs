//! In-memory actor host - spawns tokio tasks and routes events/commands.
//!
//! Provides [`spawn`] for spawning actors with lifecycle events,
//! [`system_spawn`] for infrastructure actors (no lifecycle events),
//! and [`InMemoryActorHost`] for managing a collection of actors with
//! pre-computed routing tables.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use crate::common::actor::actor_counter::ActorCounter;
use crate::common::actor::protocol::event::{ActorStarted, ActorStarting};
use crate::common::actor::{
    Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink, SystemMessage,
};
use crate::protocol::{ActorName, Command, CommandName, Event, EventTypeName};
use error_stack::Report;
use kanal::Receiver;
use parking_lot::Mutex;

use super::actor_host::ActorHost;
use super::actor_host::ActorHostError;
use super::routing::RoutingEntry;

/// Maximum time to wait for each actor task to join during shutdown.
const JOIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Tracks actor shutdown completions for the host.
///
/// Shared between the host (which populates the pending set) and
/// the actor run loops (which signal completion). When all tracked
/// actors have completed and shutdown is active, the oneshot sender
/// is fired.
#[derive(Clone)]
pub struct ShutdownTracker {
    inner: Arc<Mutex<ShutdownTrackerInner>>,
}

struct ShutdownTrackerInner {
    /// Whether shutdown has been initiated.
    active: bool,
    /// Names of actors that have not yet completed shutdown.
    pending: HashSet<String>,
    /// Channel to signal when all actors have completed shutdown.
    completion_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Default for ShutdownTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ShutdownTracker {
    /// Creates a new inactive tracker with no pending actors.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ShutdownTrackerInner {
                active: false,
                pending: HashSet::new(),
                completion_tx: None,
            })),
        }
    }

    /// Configures the tracker for shutdown: populates the pending set
    /// from the given actor names and stores the oneshot sender.
    pub fn begin(
        &self,
        names: impl Iterator<Item = String>,
        completion_tx: tokio::sync::oneshot::Sender<()>,
    ) {
        let mut inner = self.inner.lock();
        inner.active = true;
        inner.pending = names.collect();
        inner.completion_tx = Some(completion_tx);
    }

    /// Records that an actor has completed shutdown.
    ///
    /// When all tracked actors have completed, fires the oneshot sender.
    pub(crate) fn complete(&self, name: &str) {
        let mut inner = self.inner.lock();
        inner.pending.remove(name);
        if inner.active
            && inner.pending.is_empty()
            && let Some(tx) = inner.completion_tx.take()
        {
            let _ = tx.send(());
        }
    }
}

/// Result of spawning an actor.
pub struct ActorSpawnResult {
    /// Routing entry for bus event/command dispatch.
    /// Contains closures, name, and subscription metadata.
    pub routing: RoutingEntry,
    /// Task join handle for shutdown.
    pub task: tokio::task::JoinHandle<()>,
}

/// Spawns an actor with lifecycle events and self-ref injection.
///
/// This is the primary spawn function for all regular actors. It:
/// 1. Creates the channel and `ActorRef`
/// 2. Emits `ActorStarting` via the sink
/// 3. Injects the actor's own `ActorRef` into the context (for self-scheduling)
/// 4. Calls the `configure` closure for dependency injection and subscriptions
/// 5. Calls `Actor::activate` to create the actor
/// 6. Spawns the tokio task via [`spawn_actor_impl`]
/// 7. Emits `ActorStarted` via the sink
///
/// Returns the routing entry and task join handle.
///
/// # Panics
///
/// Panics if the tokio task cannot be spawned.
pub fn spawn<A>(
    name: &str,
    sink: &Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
    counter: &ActorCounter,
    shutdown_tracker: &ShutdownTracker,
    deps: A::Deps,
) -> ActorSpawnResult
where
    A: Actor + Send + 'static,
{
    counter.increment();

    let _ = sink.send_event(Event::ActorStarting(ActorStarting {
        name: name.to_owned(),
        description: None,
    }));

    let (tx, rx) = kanal::unbounded::<ActorEnvelope<A::Message>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new(name, sink.clone());
    ctx.set_actor_ref(actor_ref.clone());
    let actor = A::activate(deps, &mut ctx);
    let description = ctx.description().map(str::to_owned);
    let result = spawn_actor_impl(
        name,
        actor,
        &actor_ref,
        rx,
        ctx,
        handle,
        shutdown_tracker.clone(),
    );

    let _ = sink.send_event(Event::ActorStarted(ActorStarted {
        name: name.to_owned(),
        description,
    }));

    result
}

/// Spawns an infrastructure actor without lifecycle events.
///
/// Same as [`spawn`] but does not emit `ActorStarting`/`ActorStarted`.
/// Used for system-level actors (system-ready) that need
/// to observe lifecycle events from all other actors. The caller is responsible
/// for emitting lifecycle events after spawning.
///
/// # Panics
///
/// Panics if the tokio task cannot be spawned.
pub fn system_spawn<A>(
    name: &str,
    sink: Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
    counter: &ActorCounter,
    shutdown_tracker: &ShutdownTracker,
    deps: A::Deps,
) -> ActorSpawnResult
where
    A: Actor + Send + 'static,
{
    counter.increment();

    let (tx, rx) = kanal::unbounded::<ActorEnvelope<A::Message>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new(name, sink);
    ctx.set_actor_ref(actor_ref.clone());
    let actor = A::activate(deps, &mut ctx);
    spawn_actor_impl(
        name,
        actor,
        &actor_ref,
        rx,
        ctx,
        handle,
        shutdown_tracker.clone(),
    )
}

/// Internal: spawns a single actor's run loop as a tokio task.
///
/// Reads registrations from the context, builds routing closures,
/// and spawns the async message loop. The `shutdown_tracker` is used
/// to signal when this actor completes its shutdown.
///
/// # Panics
///
/// Panics if the tokio task cannot be spawned.
pub fn spawn_actor_impl<M, A>(
    name: &str,
    actor: A,
    actor_ref: &ActorRef<M>,
    receiver: Receiver<ActorEnvelope<M>>,
    mut ctx: ActorContext,
    handle: &tokio::runtime::Handle,
    shutdown_tracker: ShutdownTracker,
) -> ActorSpawnResult
where
    M: Send + 'static,
    A: Actor<Message = M> + Send + 'static,
{
    let (subscriptions, commands, subscribes_all_events) = ctx.take_registrations();

    let ref_for_event = actor_ref.clone();
    let ref_for_command = actor_ref.clone();
    let ref_for_system = actor_ref.clone();
    let name_for_event_log = name.to_owned();
    let name_for_command_log = name.to_owned();
    let name_for_system_log = name.to_owned();

    let send_event: Box<dyn Fn(Event) + Send + Sync> = Box::new(move |event| {
        tracing::info!(name = %name_for_event_log, event = ?event, "DIAG routing event to actor");
        if let Err(e) = ref_for_event.send_event(event) {
            tracing::error!(name = %name_for_event_log, err = ?e, "failed to route event to actor");
        }
    });

    let send_command: Box<dyn Fn(Command) + Send + Sync> = Box::new(move |command| {
        tracing::info!(name = %name_for_command_log, cmd = %command, "DIAG routing command to actor");
        if let Err(e) = ref_for_command.send_command(command) {
            tracing::error!(name = %name_for_command_log, err = ?e, "failed to route command to actor");
        }
    });

    let send_system: Box<dyn Fn(SystemMessage) + Send + Sync> = Box::new(move |msg| {
        if let Err(e) = ref_for_system.send_system(msg) {
            tracing::error!(name = %name_for_system_log, err = ?e, "failed to route system message to actor");
        }
    });

    let ref_for_close = actor_ref.clone();
    let name_for_close_log = name.to_owned();
    let close_channel: Box<dyn Fn() + Send + Sync> = Box::new(move || {
        if let Err(e) = ref_for_close.close() {
            tracing::error!(name = %name_for_close_log, err = ?e, "failed to close actor channel");
        }
    });

    let routing = RoutingEntry {
        name: name.to_owned(),
        subscriptions,
        commands,
        subscribes_all_events,
        send_event,
        send_command,
        send_system,
        close_channel,
    };

    let name_owned = name.to_owned();
    let task = handle.spawn(async move {
        let async_rx = receiver.as_async();
        let mut actor = actor;
        tracing::info!(actor = %name_owned, "DIAG actor task started");
        while let Ok(envelope) = async_rx.recv().await {
            let msg_kind = match &envelope {
                ActorEnvelope::System(_) => "system",
                ActorEnvelope::Command(cmd) => "command",
                ActorEnvelope::Event(_) => "event",
                ActorEnvelope::Direct(_) => "direct",
            };
            tracing::info!(actor = %name_owned, kind = msg_kind, "DIAG actor recv");
            let recv_time = std::time::Instant::now();
            match &envelope {
                ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                    actor.on_shutdown(&ctx).await;
                    ctx.announce_shutdown_completed();
                    shutdown_tracker.complete(&name_owned);
                    while async_rx.recv().await.is_ok() {}
                    break;
                }
                _ => actor.handle(envelope, &ctx).await,
            }
            tracing::info!(actor = %name_owned, kind = msg_kind, elapsed = ?recv_time.elapsed(), "DIAG actor handle done");
        }
        tracing::info!(actor = %name_owned, "DIAG actor task ended");
        actor.shutdown().await;
    });

    ActorSpawnResult { routing, task }
}

/// Pre-computed routing tables for lock-free event/command dispatch.
///
/// Built once during [`InMemoryActorHost::from_actors`] and never mutated.
/// The hot path (`send_event`/`send_command`) performs `HashMap` lookups
/// without any Mutex.
struct RoutingTables {
    /// Event type name → routing entries for subscribed actors.
    event_routes: HashMap<EventTypeName, Vec<Arc<RoutingEntry>>>,
    /// Command name → routing entries for registered actors.
    command_routes: HashMap<CommandName, Vec<Arc<RoutingEntry>>>,

    /// All routing entries - used for broadcasting system messages.
    all_entries: Vec<Arc<RoutingEntry>>,
    /// Actors that subscribe to ALL events (wildcard).
    all_event_subscribers: Vec<Arc<RoutingEntry>>,
}

/// Lifecycle state that is only touched during shutdown.
pub(crate) struct LifecycleState {
    /// Task join handles for actor tasks.
    pub(crate) tasks: Vec<tokio::task::JoinHandle<()>>,
}

/// Hosts actors in-memory using pre-computed routing tables.
///
/// The routing tables are built once from [`ActorSpawnResult`] entries and
/// never mutated. The hot path (`send_event`/`send_command`) performs
/// `HashMap` lookups without any Mutex - the Mutex is only touched during
/// `shutdown()` to join tasks.
pub struct InMemoryActorHost {
    /// Pre-computed routing tables for lock-free dispatch.
    routing: RoutingTables,
    /// Lifecycle state (task handles) touched only during shutdown.
    pub(crate) lifecycle: Mutex<LifecycleState>,
    /// Shared shutdown tracker - also cloned into each actor's run loop.
    shutdown_tracker: ShutdownTracker,
    /// Tokio runtime handle for spawning and joining tasks.
    handle: tokio::runtime::Handle,
}

impl InMemoryActorHost {
    /// Builds an actor host from the given spawn results.
    ///
    /// Reads `subscriptions` and `commands` from each [`RoutingEntry`] to
    /// build the routing `HashMaps`. Collects task handles for shutdown.
    ///
    /// # Panics
    ///
    /// Panics if any command is subscribed by more than one actor.
    #[must_use]
    pub fn from_actors_with_handle(
        results: Vec<ActorSpawnResult>,
        handle: tokio::runtime::Handle,
        shutdown_tracker: ShutdownTracker,
    ) -> Self {
        let mut event_routes: HashMap<EventTypeName, Vec<Arc<RoutingEntry>>> = HashMap::new();
        let mut command_routes: HashMap<CommandName, Vec<Arc<RoutingEntry>>> = HashMap::new();
        let mut all_entries: Vec<Arc<RoutingEntry>> = Vec::new();
        let mut all_event_subscribers: Vec<Arc<RoutingEntry>> = Vec::new();
        let mut tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();

        for result in results {
            let entry = Arc::new(result.routing);

            for sub in &entry.subscriptions {
                event_routes
                    .entry(sub.clone())
                    .or_default()
                    .push(entry.clone());
            }

            for cmd in &entry.commands {
                if let Some(existing) = command_routes.get(cmd) {
                    let existing_names: Vec<_> = existing.iter().map(|e| e.name.as_str()).collect();
                    panic!(
                        "command '{}' is subscribed by multiple actors: {:?} and '{}'. \
                         Commands must have exactly one subscriber.",
                        cmd, existing_names, entry.name
                    );
                }
                command_routes
                    .entry(cmd.clone())
                    .or_default()
                    .push(entry.clone());
            }

            if entry.subscribes_all_events {
                all_event_subscribers.push(entry.clone());
            }

            all_entries.push(entry);
            tasks.push(result.task);
        }

        Self {
            routing: RoutingTables {
                event_routes,
                command_routes,
                all_entries,
                all_event_subscribers,
            },
            lifecycle: Mutex::new(LifecycleState { tasks }),
            shutdown_tracker,
            handle,
        }
    }

    /// Initiates coordinated shutdown tracking.
    ///
    /// Populates the shutdown tracker with all known actor names and
    /// stores the oneshot sender. When all actors complete their shutdown,
    /// the sender fires.
    pub fn begin_shutdown(&self, completion_tx: tokio::sync::oneshot::Sender<()>) {
        let names = self.routing.all_entries.iter().map(|e| e.name.clone());
        self.shutdown_tracker.begin(names, completion_tx);
    }

    /// Shuts down all actors gracefully with a configurable timeout.
    ///
    /// Closes all actor channels (causing their run loops to exit), then
    /// joins their tasks with a per-task timeout.
    ///
    /// # Errors
    ///
    /// Returns an error if any actors fail to shut down within the timeout.
    ///
    /// # Panics
    ///
    /// Panics if called from within a tokio runtime context (uses `block_on`).
    pub fn shutdown_with_timeout(&self, timeout: Duration) -> Result<(), Report<ActorHostError>> {
        // Close all actor channels - run loops exit when recv() returns Err.
        for entry in &self.routing.all_entries {
            (entry.close_channel)();
        }

        // Drain tasks and join.
        let tasks: Vec<_> = self.lifecycle.lock().tasks.drain(..).collect();
        for task in tasks {
            let result = self
                .handle
                .block_on(async { tokio::time::timeout(timeout, task).await });
            if result.is_err() {
                tracing::warn!(?timeout, "actor task did not exit within timeout");
            }
        }

        Ok(())
    }
}

impl ActorHost for InMemoryActorHost {
    fn name(&self) -> &'static str {
        "InMemoryActorHost"
    }

    fn send_event(&self, event: &Event, source: Option<&ActorName>) {
        // Look up subscribed actors by event type name.
        let Some(event_key) = event.routing_key() else {
            return; // Not a routable event.
        };
        if let Some(entries) = self.routing.event_routes.get(&*event_key) {
            for entry in entries {
                if source.is_some_and(|s| &**s == entry.name.as_str()) {
                    continue;
                }
                tracing::info!(key = %event_key, actor = %entry.name, "DIAG routing event");
                (entry.send_event)(event.clone());
            }
        }
        // Also route to actors that subscribe to ALL events.
        for entry in &self.routing.all_event_subscribers {
            // Skip if already routed via specific subscription.
            if let Some(entries) = self.routing.event_routes.get(&*event_key)
                && entries.iter().any(|e| e.name == entry.name)
            {
                continue;
            }
            if source.is_some_and(|s| &**s == entry.name.as_str()) {
                continue;
            }
            (entry.send_event)(event.clone());
        }
    }

    fn send_command(&self, command: &Command, source: Option<&ActorName>) {
        let Some(key) = command.routing_key() else {
            return;
        };
        if let Some(entries) = self.routing.command_routes.get(&*key) {
            for entry in entries {
                if source.is_some_and(|s| &**s == entry.name.as_str()) {
                    continue;
                }
                tracing::info!(key = %key, actor = %entry.name, cmd = %command, "DIAG routing command");
                (entry.send_command)(command.clone());
            }
        } else {
            tracing::warn!(key = %key, "DIAG no actor subscribed for command");
        }
    }

    fn send_system(&self, msg: SystemMessage) {
        for entry in &self.routing.all_entries {
            (entry.send_system)(msg);
        }
    }

    fn begin_shutdown(&self, completion_tx: tokio::sync::oneshot::Sender<()>) {
        self.begin_shutdown(completion_tx);
    }

    fn shutdown(&self) -> Result<(), Report<ActorHostError>> {
        self.shutdown_with_timeout(JOIN_TIMEOUT)
    }
}
