//! MCP lifecycle actor — spawns and kills `McpActor`s.
//!
//! One instance lives for the whole app. It watches session lifecycle events
//! and the per-session enablement command, keeping exactly one `McpActor` alive
//! per (session × enabled-server) pair:
//!
//! - [`SessionLoadCompleted`] — a session restored from disk; spawn actors for
//!   its persisted `enabled_mcp_servers`.
//! - [`SessionCreated`] — a freshly created session (defaults to no MCP, but
//!   the handler re-reads enablement defensively).
//! - [`McpEnablementChanged`] — the picker committed a new desired set; diff
//!   against the spawned map and spawn/kill the delta.
//! - [`SessionClosed`] / [`SessionArchived`] / [`SessionTeardownFinished`] —
//!   the session is gone; kill all its actors.
//!
//! Each `McpActor` is a supervised child of the root supervisor
//! ([`kameo::Actor::supervise`]) with [`RestartPolicy::Never`], so a single
//! dead server's crash never cascades and never restarts (the user re-enables
//! it). Disabling a server (or closing the session) calls
//! [`ActorRef::stop_gracefully`], which triggers the `McpActor::on_stop` hook
//! that shuts the child process down.

pub mod protocol;


use std::collections::{BTreeSet, HashMap};

use kameo::actor::{ActorRef, Spawn};
use kameo::prelude::{Context, Message};
use kameo::supervision::RestartPolicy;
use parking_lot::Mutex;

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::root_supervisor::RootSupervisorRef;
use crate::common::services::bus_service::BusService;
use crate::feat::mcp::McpServerConfig;
use crate::feat::mcp_actor::{McpActor, McpActorDeps};
use crate::feat::mcp_lifecycle_actor::protocol::{McpEnablementChanged, RestartMcpServer};
use crate::feat::session::protocol::session_archived::SessionArchived;
use crate::feat::session::protocol::session_closed::SessionClosed;
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::feat::session_lifecycle::protocol::event::{
    SessionCreated, SessionTeardownFinished,
};
use crate::protocol::SessionId;
use crate::Services;

/// Key into the spawned-actor map: one `McpActor` per (session × server).
type SpawnKey = (SessionId, String);

/// The MCP lifecycle actor.
pub struct McpLifecycleActor {
    deps: ActorDeps,
    root: RootSupervisorRef,
    /// Tracks every live `McpActor` by (session_id, server_name).
    /// Guarded by a mutex so spawn/kill helpers can borrow `self` while
    /// mutating the map without fighting the borrow checker.
    spawned: Mutex<HashMap<SpawnKey, ActorRef<McpActor>>>,
}

/// Dependencies for [`McpLifecycleActor`].
#[derive(Clone)]
pub struct McpLifecycleActorDeps {
    /// Common actor dependencies (services + bus).
    pub deps: ActorDeps,
    /// Root supervisor — `McpActor`s are supervised children of it.
    pub root: RootSupervisorRef,
}

impl kameo::Actor for McpLifecycleActor {
    type Args = McpLifecycleActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.clone().recipient::<SessionLoadCompleted>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<SessionCreated>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<McpEnablementChanged>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<SessionClosed>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<SessionArchived>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<SessionTeardownFinished>())
            .await;
        args.deps
            .subscribe(actor_ref.recipient::<RestartMcpServer>())
            .await;

        Ok(Self {
            deps: args.deps,
            root: args.root,
            spawned: Mutex::new(HashMap::new()),
        })
    }
}

impl BusPublish for McpLifecycleActor {
    fn bus(&self) -> &BusService {
        &self.deps.services.bus
    }
}

impl McpLifecycleActor {
    /// Reconciles the spawned-actor map for one session against a desired set.
    ///
    /// Spawns actors for newly-enabled servers, kills actors for
    /// newly-disabled servers. Idempotent: calling with the current set is a
    /// no-op.
    async fn reconcile(&self, session_id: &SessionId, desired: &BTreeSet<String>) {
        let configs = configured_servers(&self.deps.services);

        // Partition the current spawned entries for this session into
        // to-keep and to-kill, based on the desired set.
        let to_kill: Vec<SpawnKey> = {
            let spawned = self.spawned.lock();
            spawned
                .keys()
                .filter(|(sid, _)| sid == session_id)
                .filter(|(_, server)| !desired.contains(server))
                .cloned()
                .collect()
        };

        // Spawn any desired server not yet running. Guarded against duplicate
        // spawns: if an entry already exists for this key, skip it.
        let to_spawn: Vec<String> = {
            let spawned = self.spawned.lock();
            desired
                .iter()
                .filter(|server| !spawned.contains_key(&(session_id.clone(), (*server).clone())))
                .cloned()
                .collect()
        };

        for server in to_spawn {
            if let Some(config) = configs.iter().find(|c| c.name == server) {
                self.spawn_one(session_id, config).await;
            } else {
                tracing::warn!(
                    server = %server,
                    %session_id,
                    "MCP lifecycle: enabled server not found in jinn.toml [[mcp_servers]], skipping spawn"
                );
            }
        }

        for key in to_kill {
            self.kill_one(&key).await;
        }
    }

    /// Spawns a single `McpActor` for a (session, server) pair and records it.
    async fn spawn_one(&self, session_id: &SessionId, config: &McpServerConfig) {
        let key = (session_id.clone(), config.name.clone());
        // Duplicate-spawn guard: another in-flight reconcile may have inserted
        // this key between the snapshot and now.
        if self.spawned.lock().contains_key(&key) {
            return;
        }

        let actor_ref = McpActor::supervise(
            &self.root,
            McpActorDeps {
                deps: self.deps.clone(),
                session_id: session_id.clone(),
                server: config.clone(),
            },
        )
        .restart_policy(RestartPolicy::Never)
        .spawn()
        .await;

        self.spawned.lock().insert(key, actor_ref);
        tracing::info!(
            server = %config.name,
            %session_id,
            "MCP lifecycle: spawned McpActor"
        );
    }

    /// Stops a single tracked `McpActor` and removes it from the map.
    ///
    /// `stop_gracefully` triggers `McpActor::on_stop`, which shuts the child
    /// process down.
    async fn kill_one(&self, key: &SpawnKey) {
        let actor_ref = self.spawned.lock().remove(key);
        if let Some(actor_ref) = actor_ref {
            let _ = actor_ref.stop_gracefully().await;
            tracing::info!(
                server = %key.1,
                session_id = %key.0,
                "MCP lifecycle: stopped McpActor"
            );
        }
    }

    /// Kills every `McpActor` for a session (used on close/archive/teardown).
    async fn kill_all_for_session(&self, session_id: &SessionId) {
        let keys: Vec<SpawnKey> = self
            .spawned
            .lock()
            .keys()
            .filter(|(sid, _)| sid == session_id)
            .cloned()
            .collect();
        for key in keys {
            self.kill_one(&key).await;
        }
    }

    /// Restarts a single (session × server) `McpActor`: kills the running one
    /// (if any) and respawns it from its configured server entry.
    ///
    /// No-op if the server isn't configured in `[[mcp_servers]]` — there's
    /// nothing valid to respawn from. This mirrors the future dashboard
    /// "restart" capability: recover a wedged process without an
    /// enable/disable round-trip through the picker.
    async fn restart_one(&self, session_id: &SessionId, server: &str) {
        let key = (session_id.clone(), server.to_owned());
        self.kill_one(&key).await;

        let configs = configured_servers(&self.deps.services);
        if let Some(config) = configs.iter().find(|c| c.name == server) {
            self.spawn_one(session_id, config).await;
        } else {
            tracing::warn!(
                server = %server,
                %session_id,
                "MCP lifecycle: restart requested for unknown server, ignoring"
            );
        }
    }
}

/// Reads the configured `[[mcp_servers]]` from user preferences.
fn configured_servers(services: &Services) -> Vec<McpServerConfig> {
    let prefs = services.user_preferences_storage.read();
    prefs.mcp_servers.clone()
}


// ── Message handlers ─────────────────────────────────────────────────────

impl Message<SessionLoadCompleted> for McpLifecycleActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: SessionLoadCompleted,
        _ctx: &mut Context<Self, Self::Reply>,
    ) {
        // Given a session restored from disk.
        let session_id = msg.session.session_id().clone();
        let enabled = msg.session.enabled_mcp_servers().clone();

        // When reconciling its enablement.
        self.reconcile(&session_id, &enabled).await;
    }
}

impl Message<SessionCreated> for McpLifecycleActor {
    type Reply = ();

    async fn handle(&mut self, msg: SessionCreated, _ctx: &mut Context<Self, Self::Reply>) {
        // New sessions are created with an empty `enabled_mcp_servers` set,
        // so there is nothing to spawn here. Restored sessions arrive via
        // `SessionLoadCompleted`; live toggles arrive via `McpEnablementChanged`.
        // Reconciling against an empty desired set is a no-op when nothing has
        // been spawned for this session yet.
        let empty = BTreeSet::new();
        self.reconcile(&msg.session_id, &empty).await;
    }
}

impl Message<McpEnablementChanged> for McpLifecycleActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: McpEnablementChanged,
        _ctx: &mut Context<Self, Self::Reply>,
    ) {
        // Given a new desired enablement set for a session.
        // When reconciling.
        self.reconcile(&msg.session_id, &msg.enabled).await;
    }
}

impl Message<SessionClosed> for McpLifecycleActor {
    type Reply = ();

    async fn handle(&mut self, msg: SessionClosed, _ctx: &mut Context<Self, Self::Reply>) {
        self.kill_all_for_session(&msg.session_id).await;
    }
}

impl Message<SessionArchived> for McpLifecycleActor {
    type Reply = ();

    async fn handle(&mut self, msg: SessionArchived, _ctx: &mut Context<Self, Self::Reply>) {
        self.kill_all_for_session(&msg.session_id).await;
    }
}

impl Message<SessionTeardownFinished> for McpLifecycleActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: SessionTeardownFinished,
        _ctx: &mut Context<Self, Self::Reply>,
    ) {
        self.kill_all_for_session(&msg.session_id).await;
    }
}

impl Message<RestartMcpServer> for McpLifecycleActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: RestartMcpServer,
        _ctx: &mut Context<Self, Self::Reply>,
    ) {
        self.restart_one(&msg.session_id, &msg.server).await;
    }
}

