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
use crate::feat::plugin_coordinator_actor::protocol::{
    PluginPhase, PluginStatus, PluginSubscriptions,
};

use jinn_plugin::{PluginHost, PluginReader};

#[cfg(test)]
use jinn_plugin::FakeGuestScript;
use jinn_plugin_api::{Envelope, HostToPlugin, PROTOCOL_VERSION, PluginToHost, Welcome};

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
    /// The plugin's name — the `[plugin.<name>]` table key.
    name: String,
    /// The hosted guest (writer half; the reader is pumped by a task).
    host: Option<PluginHost>,
}

/// Dependencies for [`PluginActor`].
#[derive(Clone)]
pub struct PluginActorDeps {
    /// Common actor dependencies (services + bus).
    pub deps: ActorDeps,
    /// The plugin's name — the `[plugin.<name>]` table key. Carried
    /// separately because `PluginConfig` no longer has a `name` field.
    pub name: String,
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
    /// Test seam: when set, `on_start` exchanges the production host's
    /// pipes for an in-process fake guest speaking the same wire. The
    /// production path (real wasm guest) is unaffected when `None`.
    ///
    /// The slot holds the *script* the fake guest runs; the pipes are real
    /// duplex streams, so the handshake and pump logic under test are the
    /// production code paths.
    #[cfg(test)]
    pub fake_guest: std::sync::Arc<std::sync::Mutex<Option<FakeGuestScript>>>,
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
                name: args.name.clone(),
                phase: PluginPhase::Starting,
            })
            .await;

        // Startup failure (spawn or handshake) is non-fatal to the actor:
        // absorb it as a live-but-hostless actor publishing Dead. The
        // coordinator clears its map entry from the status event — the
        // same path a guest that dies at runtime takes.
        let start_result = {
            // Test seam: a scripted in-process guest replaces the wasm
            // module. Same wire, same pipes, same codec — the handshake
            // and pump code under test cannot tell the difference.
            #[cfg(test)]
            let start = match args.fake_guest.lock().ok().and_then(|g| g.clone()) {
                Some(script) => Ok(PluginHost::fake(&args.name, script)),
                None => start_real_guest(&args),
            };
            #[cfg(not(test))]
            let start = start_real_guest(&args);

            match start {
                Ok(mut host) => match handshake(&mut host, &args).await {
                    Ok(subscriptions) => Ok((host, subscriptions)),
                    Err(error) => {
                        tracing::warn!(plugin = %args.name, error = ?error, "handshake failed");
                        Err(())
                    }
                },
                Err(error) => {
                    tracing::warn!(plugin = %args.name, error = ?error, "guest failed to start");
                    Err(())
                }
            }
        };

        let Ok((host, subscriptions)) = start_result else {
            args.deps
                .publish(PluginStatus {
                    name: args.name.clone(),
                    phase: PluginPhase::Dead,
                })
                .await;
            return Ok(Self {
                deps: args.deps,
                name: args.name,
                host: None,
            });
        };

        args.deps
            .publish(PluginStatus {
                name: args.name.clone(),
                phase: PluginPhase::Running,
            })
            .await;
        tracing::info!(plugin = %args.name, "plugin actor: handshake complete");

        // Announce the validated subscription set so the coordinator's
        // event forwarder knows what to route to this guest. Published
        // after `Running` so subscribers see a live plugin first.
        if !subscriptions.is_empty() {
            args.deps
                .publish(PluginSubscriptions {
                    name: args.name.clone(),
                    kinds: subscriptions,
                })
                .await;
        }

        // Split the reader off and pump it: every decoded envelope goes to
        // the coordinator's channel. The task ends at guest EOF, which
        // stops this actor (publishing Dead in on_stop) — the coordinator
        // observes the plugin's death via that status event.
        let mut host = host;
        spawn_read_pump(
            host.split(),
            args.name.clone(),
            args.inbound_tx.clone(),
            actor_ref.downgrade(),
        );

        Ok(Self {
            deps: args.deps,
            name: args.name,
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
                name: self.name.clone(),
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

/// Pumps the guest's stdout until EOF, forwarding every decoded envelope to
/// the coordinator's channel.
///
/// Backpressure is drop-newest: when the inbound channel is full (the
/// coordinator is slow), the incoming message is dropped rather than
/// blocking the pump. The first drop of an episode publishes
/// [`PluginPhase::Unresponsive`]; the first successful send afterwards
/// publishes [`PluginPhase::Running`] again. The pump never blocks — a
/// flooding guest degrades the plugin's own status, never the host.
fn spawn_read_pump(
    mut reader: PluginReader,
    name: String,
    inbound_tx: tokio::sync::mpsc::Sender<PluginInbound>,
    actor_ref: kameo::actor::WeakActorRef<PluginActor>,
) {
    tokio::spawn(async move {
        let mut unresponsive = false;
        while let Ok(Some(envelope)) = reader.read_next().await {
            let event = match envelope.msg {
                jinn_plugin_api::PluginToHostOrHostToPlugin::Plugin(event) => event,
                jinn_plugin_api::PluginToHostOrHostToPlugin::Host(_)
                | jinn_plugin_api::PluginToHostOrHostToPlugin::Unknown => continue,
            };
            match inbound_tx.try_send(PluginInbound {
                name: name.clone(),
                event,
            }) {
                Ok(()) => {
                    if unresponsive {
                        unresponsive = false;
                        request_phase_publish(&actor_ref, PluginPhase::Running).await;
                    }
                }
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    if !unresponsive {
                        unresponsive = true;
                        request_phase_publish(&actor_ref, PluginPhase::Unresponsive).await;
                    }
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    // Coordinator gone: stop pumping.
                    break;
                }
            }
        }
        tracing::info!(plugin = %name, "plugin read pump ended");
        // Guest EOF (or read error): the actor has nothing left to serve.
        if let Some(strong) = actor_ref.upgrade() {
            let _ = strong.stop_gracefully().await;
        }
    });
}

/// Asks the actor to publish a phase change on the bus. The pump outlives
/// any borrow of the actor's bus handle, so it forwards through the actor
/// ref; if the actor is already gone the status is moot and silently
/// dropped.
async fn request_phase_publish(
    actor_ref: &kameo::actor::WeakActorRef<PluginActor>,
    phase: PluginPhase,
) {
    if let Some(strong) = actor_ref.upgrade() {
        let _ = strong.tell(PublishPhase(phase)).await;
    }
}

/// Pump → actor request: publish this phase on the bus.
#[derive(Debug)]
struct PublishPhase(PluginPhase);

/// Starts the production wasm guest through the shared engine.
fn start_real_guest(args: &PluginActorDeps) -> Result<PluginHost, PluginActorError> {
    PluginHost::start(&args.engine, &args.name, &args.wasm_path, &args.grants).map_err(
        |report: Report<jinn_plugin::PluginHostError>| {
            tracing::warn!(plugin = %args.name, "{report:#}");
            PluginActorError::Spawn
        },
    )
}

/// Completes the v1 handshake: waits (bounded) for the guest `Hello`,
/// replies `Welcome`, and fails on timeout, version mismatch, or a
/// non-`Hello` first message.
///
/// Returns the guest's validated subscription set (unknown tags warned
/// about and dropped) so the actor can announce it for the event
/// forwarder.
async fn handshake(
    host: &mut PluginHost,
    args: &PluginActorDeps,
) -> Result<Vec<String>, PluginActorError> {
    let envelope = match tokio::time::timeout(HANDSHAKE_TIMEOUT, host.read()).await {
        Ok(Ok(Some(env))) => env,
        Ok(Ok(None)) => {
            tracing::warn!(
                plugin = %args.name,
                "plugin closed stdout before Hello"
            );
            return Err(PluginActorError::Handshake);
        }
        Ok(Err(report)) => {
            tracing::warn!(plugin = %args.name, "{report:#}");
            return Err(PluginActorError::Handshake);
        }
        Err(_) => {
            tracing::warn!(plugin = %args.name, "plugin handshake timed out");
            return Err(PluginActorError::Handshake);
        }
    };

    let jinn_plugin_api::PluginToHostOrHostToPlugin::Plugin(PluginToHost::Hello(hello)) =
        envelope.msg
    else {
        tracing::warn!(
            plugin = %args.name,
            "plugin sent non-Hello first message"
        );
        return Err(PluginActorError::Handshake);
    };
    if hello.protocol_version != PROTOCOL_VERSION {
        tracing::warn!(
            plugin = %args.name,
            version = hello.protocol_version,
            "plugin protocol version mismatch"
        );
        return Err(PluginActorError::Handshake);
    }
    let subscriptions = validate_subscriptions(&args.name, hello.subscriptions);

    let welcome = Envelope::for_host(
        HostToPlugin::Welcome(Welcome {
            protocol_version: PROTOCOL_VERSION,
            plugin_id: args.name.clone(),
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
    host.write(&welcome).await.map_err(|report| {
        tracing::warn!(plugin = %args.name, "{report:#}");
        PluginActorError::Write
    })?;
    Ok(subscriptions)
}

/// Filters a guest's declared subscriptions down to the known kinds.
///
/// Unknown tags are warned about and dropped — a newer guest's future
/// subscription kinds must not break an older host.
fn validate_subscriptions(name: &str, declared: Vec<String>) -> Vec<String> {
    let mut valid = Vec::new();
    for tag in declared {
        if jinn_plugin_api::SUBSCRIPTION_KINDS.contains(&tag.as_str()) {
            valid.push(tag);
        } else {
            tracing::warn!(plugin = %name, tag = %tag, "unknown subscription kind ignored");
        }
    }
    valid
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

/// The per-plugin host→guest event sequence counter (envelope `seq`).
///
/// Sequence numbers are per direction and per plugin; the host side counts
/// from 1 (0 was the handshake `Welcome`).
static HOST_EVENT_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Coordinator → plugin actor request: write one host event to this
/// guest's stdin.
///
/// Fire-and-forget by design: a write failure (dead guest, closed pipe)
/// is absorbed — the guest's death is surfaced through its own lifecycle
/// events, never by blocking the forwarder.
#[derive(Debug, Clone)]
pub struct DeliverHostEvent(pub jinn_plugin_api::HostToPlugin);

impl Message<DeliverHostEvent> for PluginActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: DeliverHostEvent,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let Some(host) = self.host.as_mut() else {
            return;
        };
        let seq = HOST_EVENT_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let envelope = Envelope::for_host(msg.0, seq, now_ms());
        if let Err(report) = host.write(&envelope).await {
            tracing::warn!(plugin = %self.name, "host event delivery failed: {report:#}");
        }
    }
}

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

impl Message<PublishPhase> for PluginActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: PublishPhase,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let PublishPhase(phase) = msg;
        tracing::warn!(
            plugin = %self.name,
            phase = ?phase,
            "plugin actor: pump phase change"
        );
        self.deps
            .publish(PluginStatus {
                name: self.name.clone(),
                phase,
            })
            .await;
    }
}
