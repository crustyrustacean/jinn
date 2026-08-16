//! Plugin coordinator — spawns plugin actors, owns the trust boundary and
//! the contribution cache.
//!
//! One instance lives for the whole app. At `on_start` it reads the enabled
//! `[[plugin]]` entries from `jinn.toml` and spawns one
//! [`PluginActor`](crate::feat::plugin_actor::PluginActor) per entry
//! (supervised, `RestartPolicy::Never` — a dead plugin stays dead until
//! the next app start).
//!
//! Every inbound plugin message flows through the coordinator's private
//! channel ([`PluginInbound`]). The coordinator validates and authorizes:
//! unknown variants and malformed payloads are dropped, unrequested
//! commands are ignored. Accepted contributions are translated
//! (`ThemeDef` → core `Theme`) and written into the plugin contribution
//! cache in `AppState` — the coordinator is its only writer.
//!
//! `PluginStatus` events published by the plugin actors are also received
//! here (bus subscription) and mirrored into the state cache so the UI can
//! read plugin health synchronously.
//!
//! A dead plugin means a stale cache, never a blocked or failed consumer:
//! the theme picker falls back to the built-in default theme when the
//! cache is empty (see the theme extraction phase).

use std::collections::HashMap;

use kameo::actor::{ActorRef, Spawn};
use kameo::prelude::{Context, Message};
use kameo::supervision::RestartPolicy;
use parking_lot::Mutex;
use tokio::sync::mpsc;

pub mod protocol;
pub mod translate;

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::root_supervisor::RootSupervisorRef;
use crate::common::services::bus_service::BusService;
use crate::common::state::State;
use crate::feat::plugin::PluginConfig;
use crate::feat::plugin_actor::{PluginActor, PluginActorDeps, PluginInbound};
use crate::feat::plugin_coordinator_actor::protocol::{PluginPhase, PluginStatus};

/// Channel capacity for plugin→coordinator inbound events. Small: events are
/// rare (handshake + contributions); a full channel means a flooding plugin,
/// and senders should feel backpressure.
const INBOUND_CAPACITY: usize = 64;

/// The plugin coordinator actor.
pub struct PluginCoordinatorActor {
    deps: ActorDeps,
    root: RootSupervisorRef,
    state: State,
    cap: crate::common::tcaps::PluginsCap,
    /// Config dir / data dir context for grant resolution and wasm paths.
    dirs: PluginDirs,
    /// Live plugin actors by name.
    spawned: Mutex<HashMap<String, ActorRef<PluginActor>>>,
}

/// Directory context the coordinator resolves plugin paths against.
#[derive(Debug, Clone)]
pub struct PluginDirs {
    /// User config dir (e.g. `~/.config/jinn`).
    pub config_dir: std::path::PathBuf,
    /// User data dir (e.g. `~/.local/share/jinn`).
    pub data_dir: std::path::PathBuf,
    /// The shared wasmtime engine, reused across all plugin guests.
    pub engine: std::sync::Arc<jinn_plugin::PluginEngine>,
}

/// Dependencies for [`PluginCoordinatorActor`].
#[derive(Clone)]
pub struct PluginCoordinatorActorDeps {
    /// Common actor dependencies (services + bus).
    pub deps: ActorDeps,
    /// Root supervisor — plugin actors are supervised children of it.
    pub root: RootSupervisorRef,
    /// Shared application state — the contribution cache lives here.
    pub state: State,
    /// Authority to write the plugin contribution cache.
    pub cap: crate::common::tcaps::PluginsCap,
    /// Directory context for grant resolution and wasm paths.
    pub dirs: PluginDirs,
}

impl kameo::Actor for PluginCoordinatorActor {
    type Args = PluginCoordinatorActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(
        args: Self::Args,
        actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.recipient::<PluginStatus>())
            .await;

        let actor = Self {
            deps: args.deps,
            root: args.root,
            state: args.state,
            cap: args.cap,
            dirs: args.dirs,
            spawned: Mutex::new(HashMap::new()),
        };

        // Spawn every enabled plugin from jinn.toml.
        actor.spawn_all().await;

        Ok(actor)
    }
}

impl BusPublish for PluginCoordinatorActor {
    fn bus(&self) -> &BusService {
        &self.deps.services.bus
    }
}

impl PluginCoordinatorActor {
    /// Spawns a plugin actor for every enabled `[[plugin]]` entry.
    async fn spawn_all(&self) {
        let configs = enabled_plugins(&self.deps.services);
        for config in configs {
            self.spawn_one(&config).await;
        }
    }

    /// Spawns one plugin actor (idempotent per name) with its inbound
    /// channel and grant set resolved from the manifest entry.
    async fn spawn_one(&self, config: &PluginConfig) {
        if self.spawned.lock().contains_key(&config.name) {
            return;
        }

        let (inbound_tx, mut inbound_rx) = mpsc::channel::<PluginInbound>(INBOUND_CAPACITY);

        let grants = match self.resolve_grants(config) {
            Ok(grants) => grants,
            Err(report) => {
                tracing::warn!(plugin = %config.name, "{report:#}");
                self.deps
                    .publish(PluginStatus {
                        name: config.name.clone(),
                        phase: PluginPhase::Dead,
                    })
                    .await;
                return;
            }
        };

        let wasm_path = self.wasm_path(config);
        let actor_ref = PluginActor::supervise(
            &self.root,
            PluginActorDeps {
                deps: self.deps.clone(),
                config: config.clone(),
                grants,
                wasm_path,
                engine: self.dirs.engine.clone(),
                inbound_tx: inbound_tx.clone(),
            },
        )
        .restart_policy(RestartPolicy::Never)
        .spawn()
        .await;
        self.spawned
            .lock()
            .insert(config.name.clone(), actor_ref);
        tracing::info!(plugin = %config.name, "plugin coordinator: spawned PluginActor");

        // The coordinator's inbound pump: every message from this plugin is
        // validated here before it may touch state.
        let state = self.state.clone();
        let cap = self.cap;
        let name = config.name.clone();
        tokio::spawn(async move {
            while let Some(inbound) = inbound_rx.recv().await {
                handle_inbound(&state, cap, &name, inbound);
            }
        });
    }

    /// Resolves manifest grants into runner grants against the dir context.
    fn resolve_grants(
        &self,
        config: &PluginConfig,
    ) -> Result<jinn_plugin::Grants, error_stack::Report<jinn_plugin::GrantsError>> {
        let ctx = jinn_plugin::DirContext {
            config_dir: self.dirs.config_dir.clone(),
            data_dir: self.dirs.data_dir.clone(),
            plugin_name: config.name.clone(),
        };
        let path_grants: Vec<jinn_plugin::PathGrant> = config
            .grants
            .iter()
            .map(|g| jinn_plugin::PathGrant {
                path: g.path.clone(),
                writable: g.writable,
            })
            .collect();
        let config_value = config
            .config
            .clone()
            .map_or(serde_json::Value::Null, |v| serde_json::to_value(&v).unwrap_or(serde_json::Value::Null));
        jinn_plugin::resolve_grants(&path_grants, config.http, config_value, &ctx)
    }

    /// Resolves a plugin's wasm path: absolute, or relative to the plugins dir.
    fn wasm_path(&self, config: &PluginConfig) -> std::path::PathBuf {
        let candidate = std::path::PathBuf::from(&config.wasm);
        if candidate.is_absolute() {
            candidate
        } else {
            self.dirs.data_dir.join("plugins").join(candidate)
        }
    }
}

/// Reads the enabled plugin entries from user preferences.
fn enabled_plugins(services: &crate::Services) -> Vec<PluginConfig> {
    services
        .user_preferences_storage
        .read()
        .plugin
        .iter()
        .filter(|p| p.enabled)
        .cloned()
        .collect()
}

/// Validates and applies one inbound plugin message.
///
/// The trust boundary: nothing from a plugin reaches `AppState` except
/// through this function's match. Unknown variants are silently dropped
/// (forward compatibility); malformed theme payloads are dropped with a
/// warn. `Hello` outside the handshake is ignored.
fn handle_inbound(state: &State, cap: crate::common::tcaps::PluginsCap, name: &str, inbound: PluginInbound) {
    match inbound.event {
        jinn_plugin_api::PluginToHost::Hello(_) => {
            // Handshake was already completed by the actor; a second Hello
            // is protocol noise — ignore it.
        }
        jinn_plugin_api::PluginToHost::SetThemeEntries(entries) => {
            let themes = crate::feat::plugin_coordinator_actor::translate::themes(&entries.themes);
            if themes.is_empty() && !entries.themes.is_empty() {
                tracing::warn!(plugin = %name, "all theme definitions failed translation");
            }
            state.with_plugins(&cap, |p| p.set_themes(name, themes));
        }
    }
}

impl Message<PluginStatus> for PluginCoordinatorActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: PluginStatus,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let PluginStatus { name, phase } = msg;
        self.state.with_plugins(&self.cap, |p| p.set_phase(name.clone(), phase));
        // A dead plugin stays dead until the next app start
        // (RestartPolicy::Never); drop the actor ref so a future
        // reconciliation could respawn (v1: nothing reconciles).
        if phase == PluginPhase::Dead {
            self.spawned.lock().remove(&name);
        }
    }
}
