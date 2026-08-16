//! Plugin guest actor — owns one hosted plugin end to end.
//!
//! Spawned (supervised, `RestartPolicy::Never`) by the plugin coordinator
//! ([`crate::feat::plugin_coordinator_actor`]) for each enabled
//! `[[plugin]]` entry at app start. This actor:
//!
//! - starts the in-process guest ([`PluginHost`] — one wasm store on a
//!   spawned task, stdio over in-memory pipes),
//! - completes the v1 handshake: waits for the guest `Hello`, replies
//!   `Welcome` (plugin name, resolved grant dirs, config),
//! - spawns the read pump: a task that forwards every decoded inbound
//!   envelope to the coordinator's private channel (the coordinator owns
//!   the trust boundary and the contribution cache — this actor never
//!   writes `AppState`),
//! - dies with the guest: guest EOF ends the read pump, which stops the
//!   actor, publishing `Dead` in `on_stop`. `ShutdownPlugin` aborts the
//!   guest task first.

use error_stack::Report;
use kameo::prelude::{Context, Message};

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::feat::plugin_coordinator_actor::protocol::{PluginPhase, PluginStatus};

use jinn_plugin::{PluginHost, PluginReader};
use jinn_plugin_api::{Envelope, HostToPlugin, PluginToHost, Welcome, PROTOCOL_VERSION};

/// How long the handshake waits for the guest `Hello` before declaring the
/// plugin dead. Guests booting a wasm runtime legitimately take a moment;
/// a wedged guest must not stall startup forever.
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// How long a graceful shutdown waits before aborting the guest task.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// One inbound message from a plugin, tagged with its source.
///
/// Sent over the plugin actor's private channel to the coordinator;
/// the coordinator is the only trust boundary.
#[derive(Debug)]
pub struct PluginInbound {
    /// The sending plugin's manifest name.
    pub name: String,
    /// The decoded wire event.
    pub event: PluginToHost,
}

/// Errors that end a plugin actor's life.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub enum PluginActorError {
    /// The guest could not be loaded or instantiated.
    Spawn,
    /// The handshake failed (timeout, malformed `Hello`, or EOF first).
    Handshake,
    /// Writing to the guest failed (line cap exceeded, closed pipe).
    Write,
}

/// The per-plugin actor.
pub struct PluginActor {
    deps: ActorDeps,
    /// Manifest entry for this plugin.
    config: crate::feat::plugin::PluginConfig,
    /// The hosted guest (writer half; the reader is pumped by a task).
    host: Option<PluginHost>,
}

/// Dependencies for [`PluginActor`].
#[derive(Clone)]
pub struct PluginActorDeps {
    /// Common actor dependencies (services + bus).
    pub deps: ActorDeps,
    /// The manifest entry this actor serves.
    pub config: crate::feat::plugin::PluginConfig,
    /// Resolved capability grants for the guest.
    pub grants: jinn_plugin::Grants,
    /// Absolute path to the plugin `.wasm` file.
    pub wasm_path: std::path::PathBuf,
    /// The shared wasmtime engine (one per process, reused per plugin).
    pub engine: std::sync::Arc<jinn_plugin::PluginEngine>,
    /// Coordinator's inbound-event channel.
    pub inbound_tx: tokio::sync::mpsc::Sender<PluginInbound>,
}

impl kameo::Actor for PluginActor {
    type Args = PluginActorDeps;
    type Error = PluginActorError;

    async fn on_start(
        args: Self::Args,
        actor_ref: kameo::actor::ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        // Announce Starting so bus subscribers see the transition even if
        // the spawn immediately fails.
        args.deps
            .publish(PluginStatus {
                name: args.config.name.clone(),
                phase: PluginPhase::Starting,
            })
            .await;

        let mut host = PluginHost::start(
            &args.engine,
            &args.config.name,
            &args.wasm_path,
            &args.grants,
        )
        .map_err(|report: Report<jinn_plugin::PluginHostError>| {
            tracing::warn!(plugin = %args.config.name, "{report:#}");
            PluginActorError::Spawn
        })?;

        handshake(&mut host, &args).await?;

        args.deps
            .publish(PluginStatus {
                name: args.config.name.clone(),
                phase: PluginPhase::Running,
            })
            .await;
        tracing::info!(plugin = %args.config.name, "plugin actor: handshake complete");

        // Split the reader off and pump it: every decoded envelope goes to
        // the coordinator's channel. The task ends at guest EOF, which
        // stops this actor (publishing Dead in on_stop) — the coordinator
        // observes the plugin's death via that status event.
        spawn_read_pump(
            host.split(),
            args.config.name.clone(),
            args.inbound_tx.clone(),
            actor_ref.downgrade(),
        );

        Ok(Self {
            deps: args.deps,
            config: args.config,
            host: Some(host),
        })
    }

    async fn on_stop(
        &mut self,
        _actor_ref: kameo::actor::WeakActorRef<Self>,
        _reason: kameo::error::ActorStopReason,
    ) -> Result<(), Self::Error> {
        // Announce death so the coordinator clears its spawned map entry
        // and the state cache reflects the loss. Dropping the host aborts
        // the guest task; bounded graceful shutdown already ran (or never
        // applied).
        self.deps
            .publish(PluginStatus {
                name: self.config.name.clone(),
                phase: PluginPhase::Dead,
            })
            .await;
        Ok(())
    }
}

impl BusPublish for PluginActor {
    fn bus(&self) -> &crate::common::services::bus_service::BusService {
        &self.deps.services.bus
    }
}

/// Pumps the child's stdout until EOF, forwarding every decoded envelope to
/// the coordinator's channel.
fn spawn_read_pump(
    mut reader: PluginReader,
    name: String,
    inbound_tx: tokio::sync::mpsc::Sender<PluginInbound>,
    actor_ref: kameo::actor::WeakActorRef<PluginActor>,
) {
    tokio::spawn(async move {
        while let Ok(Some(envelope)) = reader.read_next().await {
            if inbound_tx
                .send(PluginInbound {
                    name: name.clone(),
                    event: match envelope.msg {
                        jinn_plugin_api::PluginToHostOrHostToPlugin::Plugin(event) => event,
                        jinn_plugin_api::PluginToHostOrHostToPlugin::Host(_)
                        | jinn_plugin_api::PluginToHostOrHostToPlugin::Unknown => continue,
                    },
                })
                .await
                .is_err()
            {
                // Coordinator gone: stop pumping.
                break;
            }
        }
        tracing::info!(plugin = %name, "plugin read pump ended");
        // Guest EOF (or read error): the actor has nothing left to serve.
        if let Some(strong) = actor_ref.upgrade() {
            let _ = strong.stop_gracefully().await;
        }
    });
}

/// Completes the v1 handshake: waits (bounded) for the guest `Hello`,
/// replies `Welcome`, and fails on timeout, version mismatch, or a
/// non-`Hello` first message.
async fn handshake(
    host: &mut PluginHost,
    args: &PluginActorDeps,
) -> Result<(), PluginActorError> {
    let envelope = match tokio::time::timeout(HANDSHAKE_TIMEOUT, host.read()).await {
        Ok(Ok(Some(env))) => env,
        Ok(Ok(None)) => {
            tracing::warn!(
                plugin = %args.config.name,
                "plugin closed stdout before Hello"
            );
            return Err(PluginActorError::Handshake);
        }
        Ok(Err(report)) => {
            tracing::warn!(plugin = %args.config.name, "{report:#}");
            return Err(PluginActorError::Handshake);
        }
        Err(_) => {
            tracing::warn!(plugin = %args.config.name, "plugin handshake timed out");
            return Err(PluginActorError::Handshake);
        }
    };

    let jinn_plugin_api::PluginToHostOrHostToPlugin::Plugin(PluginToHost::Hello(hello)) = envelope.msg else {
        tracing::warn!(
            plugin = %args.config.name,
            "plugin sent non-Hello first message"
        );
        return Err(PluginActorError::Handshake);
    };
    if hello.protocol_version != PROTOCOL_VERSION {
        tracing::warn!(
            plugin = %args.config.name,
            version = hello.protocol_version,
            "plugin protocol version mismatch"
        );
        return Err(PluginActorError::Handshake);
    }

    let welcome = Envelope::for_host(
        HostToPlugin::Welcome(Welcome {
            protocol_version: PROTOCOL_VERSION,
            plugin_id: args.config.name.clone(),
            read_dirs: args
                .grants
                .read_dirs
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            write_dirs: args
                .grants
                .write_dirs
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            http_allowed: args.grants.http,
            config: args.grants.config.clone(),
        }),
        0,
        now_ms(),
    );
    host.write(&welcome)
        .await
        .map_err(|report| {
            tracing::warn!(plugin = %args.config.name, "{report:#}");
            PluginActorError::Write
        })?;
    Ok(())
}

/// Unix epoch milliseconds for envelope timestamps.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Message sent to a plugin actor requesting a bounded graceful shutdown.
#[derive(Debug, Clone)]
pub struct ShutdownPlugin;

impl Message<ShutdownPlugin> for PluginActor {
    type Reply = ();

    async fn handle(
        &mut self,
        _msg: ShutdownPlugin,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if let Some(mut host) = self.host.take() {
            let _ = tokio::time::timeout(SHUTDOWN_GRACE, host.shutdown()).await;
            // Host drop aborts whatever is left; the actor then dies (no
            // guest to serve), publishing Dead in on_stop.
        }
    }
}
