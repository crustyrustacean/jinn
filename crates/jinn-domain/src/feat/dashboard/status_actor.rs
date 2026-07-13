//! The dashboard status actor — sole writer of `frontend.dashboard`.
//!
//! Combines two independent data sources into a single dashboard view:
//!
//! - **Generic actor lifecycle** — subscribes to the existing bus events
//!   [`ActorStarting`], [`ActorStarted`], and [`ActorShutdownCompleted`] to
//!   track every actor's `Starting`/`Running`/`Dead` phase.
//! - **Discord connection status** — drains a kanal channel fed by the
//!   Discord gateway task, writing the free-form status message into the
//!   discord entry's `status_message` field.
//!
//! The actor owns `frontend.dashboard` exclusively. No other code writes to
//! it.

use kameo::actor::ActorRef;
use kameo::prelude::{Actor, Context, Message};

use crate::common::actor::protocol::event::{ActorShutdownCompleted, ActorStarted, ActorStarting};
use crate::common::actor_deps::ActorDeps;
use crate::common::state::State;
use crate::feat::browser_binary_scan::{BrowserBinaryMissing, BrowserBinaryVerified};
use crate::feat::dashboard::DashboardState;

/// Dashboard entry name for the browser binary verification.
const BROWSER_BINARY_ENTRY: &str = "web-fetch-browser";
/// Description shown for the browser binary dashboard entry.
const BROWSER_BINARY_DESCRIPTION: &str = "Headless browser binary (stealth web fetcher)";

/// Discord bot-specific connection status, reported by the gateway task.
///
/// Rendered as the free-form third column of the discord dashboard entry.
/// Other actors leave `status_message` empty; only discord populates this.
#[derive(Debug, Clone)]
pub enum DiscordStatusUpdate {
    /// The gateway is attempting to connect to Discord.
    Connecting,
    /// The gateway received its `ready` event — the bot is online.
    Connected,
    /// The websocket dropped mid-session.
    Disconnected,
    /// The gateway hit a fatal error (auth failure, unresolvable disconnect).
    Error {
        /// Human-readable reason (e.g. "401: invalid bot token").
        message: String,
    },
}

impl DiscordStatusUpdate {
    /// Renders the update into the dashboard `status_message` string.
    #[must_use]
    pub fn display_message(&self) -> &'static str {
        match self {
            Self::Connecting => "Connecting",
            Self::Connected => "Connected",
            Self::Disconnected => "Disconnected",
            Self::Error { .. } => "Error",
        }
    }

    /// Returns the full human-readable detail (for the `Error` variant).
    #[must_use]
    pub fn full_message(&self) -> String {
        match self {
            Self::Error { message } => format!("Error: {message}"),
            other => other.display_message().to_owned(),
        }
    }
}

const DISCORD_ACTOR_NAME: &str = "discord";
const DISCORD_DESCRIPTION: &str = "Discord gateway bot [Task]";

/// The dashboard status actor.
///
/// Subscribes to generic lifecycle events and drains a kanal channel for
/// discord connection status. Writes all updates into `frontend.dashboard`.
pub struct DiscordStatusActor {
    state: State,
}

/// Dependencies for [`DiscordStatusActor`].
#[derive(Clone)]
pub struct DiscordStatusActorDeps {
    /// Universal actor dependencies (bus subscription handle).
    pub deps: ActorDeps,
    /// Shared application state — the dashboard sub-struct is written here.
    pub state: State,
    /// Receiver half of the kanal channel fed by the Discord gateway.
    pub status_rx: kanal::AsyncReceiver<DiscordStatusUpdate>,
}

impl Actor for DiscordStatusActor {
    type Args = DiscordStatusActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.clone().recipient::<ActorStarting>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<ActorStarted>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<ActorShutdownCompleted>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<BrowserBinaryVerified>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<BrowserBinaryMissing>())
            .await;

        // Spawn the background drain loop for the discord status channel.
        // The loop owns the receiver and a State clone; each update writes
        // directly to the dashboard.
        let drain_state = args.state.clone();
        tokio::spawn(drain_status_channel(args.status_rx, drain_state));

        Ok(Self { state: args.state })
    }
}

impl Message<ActorStarting> for DiscordStatusActor {
    type Reply = ();

    async fn handle(&mut self, msg: ActorStarting, _ctx: &mut Context<Self, Self::Reply>) {
        let mut state = self.state.write();
        state
            .frontend
            .dashboard
            .mark_starting(&msg.name, msg.description);
    }
}

impl Message<ActorStarted> for DiscordStatusActor {
    type Reply = ();

    async fn handle(&mut self, msg: ActorStarted, _ctx: &mut Context<Self, Self::Reply>) {
        let mut state = self.state.write();
        state
            .frontend
            .dashboard
            .mark_running(&msg.name, msg.description);
    }
}

impl Message<ActorShutdownCompleted> for DiscordStatusActor {
    type Reply = ();

    async fn handle(&mut self, msg: ActorShutdownCompleted, _ctx: &mut Context<Self, Self::Reply>) {
        let mut state = self.state.write();
        state.frontend.dashboard.mark_dead(&msg.name, None);
    }
}

impl Message<BrowserBinaryVerified> for DiscordStatusActor {
    type Reply = ();

    async fn handle(&mut self, msg: BrowserBinaryVerified, _ctx: &mut Context<Self, Self::Reply>) {
        let mut state = self.state.write();
        let dashboard = &mut state.frontend.dashboard;
        dashboard.mark_running(
            BROWSER_BINARY_ENTRY,
            Some(BROWSER_BINARY_DESCRIPTION.to_owned()),
        );
        dashboard.set_status_message(BROWSER_BINARY_ENTRY, Some(msg.path.display().to_string()));
    }
}

impl Message<BrowserBinaryMissing> for DiscordStatusActor {
    type Reply = ();

    async fn handle(&mut self, msg: BrowserBinaryMissing, _ctx: &mut Context<Self, Self::Reply>) {
        let mut state = self.state.write();
        let dashboard = &mut state.frontend.dashboard;
        dashboard.mark_dead(
            BROWSER_BINARY_ENTRY,
            Some(BROWSER_BINARY_DESCRIPTION.to_owned()),
        );
        dashboard.set_status_message(BROWSER_BINARY_ENTRY, Some(msg.reason));
    }
}

impl DiscordStatusActor {
    /// Apply a discord connection status update to the dashboard state.
    ///
    /// Separate from the kameo message path so it can be called from the
    /// background drain loop without an actor message round-trip.
    fn apply_discord_update(dashboard: &mut DashboardState, update: &DiscordStatusUpdate) {
        let message = update.full_message();
        let (lifecycle, with_description) = match update {
            DiscordStatusUpdate::Connecting => {
                // Ensure the discord entry exists with a description even
                // before Connected/Error arrives. The gateway task is not
                // a kameo actor, so it doesn't emit ActorStarting.
                (Some(crate::feat::dashboard::ActorLifecycle::Starting), true)
            }
            // Disconnected only updates the status message — the lifecycle
            // (Starting/Running/Dead) is driven by the other update variants.
            DiscordStatusUpdate::Disconnected => (None, false),
            DiscordStatusUpdate::Connected => {
                // The gateway task is not a kameo actor, so it doesn't emit
                // ActorStarted. Mark it running here.
                (Some(crate::feat::dashboard::ActorLifecycle::Running), true)
            }
            DiscordStatusUpdate::Error { .. } => {
                // The description is a constant for the discord entry;
                // attach it on creation even when Error arrives first
                // (e.g. missing token).
                (Some(crate::feat::dashboard::ActorLifecycle::Dead), true)
            }
        };

        if let Some(lifecycle) = lifecycle {
            let description = with_description.then(|| DISCORD_DESCRIPTION.to_owned());
            match lifecycle {
                crate::feat::dashboard::ActorLifecycle::Starting => {
                    dashboard.mark_starting(DISCORD_ACTOR_NAME, description);
                }
                crate::feat::dashboard::ActorLifecycle::Running => {
                    dashboard.mark_running(DISCORD_ACTOR_NAME, description);
                }
                crate::feat::dashboard::ActorLifecycle::Dead => {
                    dashboard.mark_dead(DISCORD_ACTOR_NAME, description);
                }
            }
        }
        dashboard.set_status_message(DISCORD_ACTOR_NAME, Some(message));
    }
}

/// Background drain loop: reads discord status updates from the kanal channel
/// and writes them into the dashboard.
async fn drain_status_channel(rx: kanal::AsyncReceiver<DiscordStatusUpdate>, state: State) {
    while let Ok(update) = rx.recv().await {
        let mut guard = state.write();
        DiscordStatusActor::apply_discord_update(&mut guard.frontend.dashboard, &update);
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::bus::test_harness::TestHarness;
    use crate::feat::dashboard::ActorLifecycle;
    use kameo::actor::Spawn;

    fn dashboard_entry(
        state: &State,
        name: &str,
    ) -> Option<(ActorLifecycle, Option<String>, Option<String>)> {
        let g = state.read();
        g.frontend
            .dashboard
            .actors()
            .iter()
            .find(|e| e.name == name)
            .map(|e| (e.lifecycle, e.status_message.clone(), e.description.clone()))
    }

    async fn spawn_actor(harness: &TestHarness, state: State) -> ActorRef<DiscordStatusActor> {
        let (_, status_rx) = kanal::unbounded::<DiscordStatusUpdate>();
        let actor = DiscordStatusActor::spawn(DiscordStatusActorDeps {
            deps: harness.actor_deps().await,
            state,
            status_rx: status_rx.to_async(),
        });
        actor.wait_for_startup().await;
        actor
    }

    #[tokio::test]
    async fn actor_starting_event_creates_entry_with_starting_lifecycle() {
        // Given a DiscordStatusActor.
        let harness = TestHarness::new().await;
        let state = State::new(AppState::default());
        spawn_actor(&harness, state.clone()).await;

        // When publishing ActorStarting.
        harness
            .publish(ActorStarting {
                name: "llm".to_owned(),
                description: None,
            })
            .await;

        // Then the dashboard shows the actor as Starting.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let (lifecycle, _, _) = dashboard_entry(&state, "llm").expect("entry should exist");
        assert_eq!(lifecycle, ActorLifecycle::Starting);
    }

    #[tokio::test]
    async fn actor_started_event_transitions_to_running() {
        // Given a DiscordStatusActor.
        let harness = TestHarness::new().await;
        let state = State::new(AppState::default());
        spawn_actor(&harness, state.clone()).await;

        // When publishing ActorStarted.
        harness
            .publish(ActorStarted {
                name: "llm".to_owned(),
                description: None,
            })
            .await;

        // Then the dashboard shows the actor as Running.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let (lifecycle, _, _) = dashboard_entry(&state, "llm").expect("entry should exist");
        assert_eq!(lifecycle, ActorLifecycle::Running);
    }

    #[tokio::test]
    async fn actor_shutdown_event_transitions_to_dead() {
        // Given a DiscordStatusActor with a running actor entry.
        let harness = TestHarness::new().await;
        let state = State::new(AppState::default());
        spawn_actor(&harness, state.clone()).await;
        harness
            .publish(ActorStarted {
                name: "llm".to_owned(),
                description: None,
            })
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // When publishing ActorShutdownCompleted.
        harness
            .publish(ActorShutdownCompleted {
                name: "llm".to_owned(),
            })
            .await;

        // Then the dashboard shows the actor as Dead.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let (lifecycle, _, _) = dashboard_entry(&state, "llm").expect("entry should exist");
        assert_eq!(lifecycle, ActorLifecycle::Dead);
    }

    #[tokio::test]
    async fn connecting_update_sets_status_message() {
        // Given a DiscordStatusActor.
        let harness = TestHarness::new().await;
        let state = State::new(AppState::default());
        let (tx, rx) = kanal::unbounded::<DiscordStatusUpdate>();
        let actor = DiscordStatusActor::spawn(DiscordStatusActorDeps {
            deps: harness.actor_deps().await,
            state: state.clone(),
            status_rx: rx.to_async(),
        });
        actor.wait_for_startup().await;

        // When sending a Connecting update.
        let _ = tx.send(DiscordStatusUpdate::Connecting);

        // Then the dashboard shows the status message.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let (_, message, _) = dashboard_entry(&state, "discord").expect("entry should exist");
        assert_eq!(message.as_deref(), Some("Connecting"));
    }

    #[tokio::test]
    async fn connected_update_marks_running_with_message() {
        // Given a DiscordStatusActor.
        let harness = TestHarness::new().await;
        let state = State::new(AppState::default());
        let (tx, rx) = kanal::unbounded::<DiscordStatusUpdate>();
        let actor = DiscordStatusActor::spawn(DiscordStatusActorDeps {
            deps: harness.actor_deps().await,
            state: state.clone(),
            status_rx: rx.to_async(),
        });
        actor.wait_for_startup().await;

        // When sending a Connected update.
        let _ = tx.send(DiscordStatusUpdate::Connected);

        // Then the dashboard shows Running + Connected.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let (lifecycle, message, _) =
            dashboard_entry(&state, "discord").expect("entry should exist");
        assert_eq!(lifecycle, ActorLifecycle::Running);
        assert_eq!(message.as_deref(), Some("Connected"));
    }

    #[tokio::test]
    async fn error_update_marks_dead_with_error_message() {
        // Given a DiscordStatusActor.
        let harness = TestHarness::new().await;
        let state = State::new(AppState::default());
        let (tx, rx) = kanal::unbounded::<DiscordStatusUpdate>();
        let actor = DiscordStatusActor::spawn(DiscordStatusActorDeps {
            deps: harness.actor_deps().await,
            state: state.clone(),
            status_rx: rx.to_async(),
        });
        actor.wait_for_startup().await;

        // When sending an Error update.
        let _ = tx.send(DiscordStatusUpdate::Error {
            message: "401: invalid token".to_owned(),
        });

        // Then the dashboard shows Dead + the error message.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let (lifecycle, message, _) =
            dashboard_entry(&state, "discord").expect("entry should exist");
        assert_eq!(lifecycle, ActorLifecycle::Dead);
        assert_eq!(message.as_deref(), Some("Error: 401: invalid token"));
    }

    #[tokio::test]
    async fn disconnected_update_keeps_status_message() {
        // Given a DiscordStatusActor with discord already connected.
        let harness = TestHarness::new().await;
        let state = State::new(AppState::default());
        let (tx, rx) = kanal::unbounded::<DiscordStatusUpdate>();
        let actor = DiscordStatusActor::spawn(DiscordStatusActorDeps {
            deps: harness.actor_deps().await,
            state: state.clone(),
            status_rx: rx.to_async(),
        });
        actor.wait_for_startup().await;
        let _ = tx.send(DiscordStatusUpdate::Connected);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // When sending a Disconnected update (mid-session drop).
        let _ = tx.send(DiscordStatusUpdate::Disconnected);

        // Then the status message updates to Disconnected.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let (_, message, _) = dashboard_entry(&state, "discord").expect("entry should exist");
        assert_eq!(message.as_deref(), Some("Disconnected"));
    }

    #[tokio::test]
    async fn error_update_first_still_sets_description() {
        // Given a DiscordStatusActor (simulating missing-token: Error arrives
        // before any Connecting/Connected update).
        let harness = TestHarness::new().await;
        let state = State::new(AppState::default());
        let (tx, rx) = kanal::unbounded::<DiscordStatusUpdate>();
        let actor = DiscordStatusActor::spawn(DiscordStatusActorDeps {
            deps: harness.actor_deps().await,
            state: state.clone(),
            status_rx: rx.to_async(),
        });
        actor.wait_for_startup().await;

        // When sending an Error update as the very first message.
        let _ = tx.send(DiscordStatusUpdate::Error {
            message: "no token configured".to_owned(),
        });

        // Then the entry is created with the discord description.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let (_, _, description) = dashboard_entry(&state, "discord").expect("entry should exist");
        assert_eq!(description.as_deref(), Some("Discord gateway bot [Task]"));
    }

    #[tokio::test]
    async fn browser_binary_verified_creates_running_entry_with_path() {
        // Given a DiscordStatusActor.
        let harness = TestHarness::new().await;
        let state = State::new(AppState::default());
        spawn_actor(&harness, state.clone()).await;

        // When publishing BrowserBinaryVerified.
        harness
            .publish(BrowserBinaryVerified {
                path: std::path::PathBuf::from("/usr/bin/google-chrome"),
            })
            .await;

        // Then the dashboard shows the browser entry as Running with the path.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let (lifecycle, message, _) =
            dashboard_entry(&state, "web-fetch-browser").expect("entry should exist");
        assert_eq!(lifecycle, ActorLifecycle::Running);
        assert_eq!(message.as_deref(), Some("/usr/bin/google-chrome"));
    }

    #[tokio::test]
    async fn browser_binary_missing_creates_dead_entry_with_reason() {
        // Given a DiscordStatusActor.
        let harness = TestHarness::new().await;
        let state = State::new(AppState::default());
        spawn_actor(&harness, state.clone()).await;

        // When publishing BrowserBinaryMissing.
        harness
            .publish(BrowserBinaryMissing {
                reason: "ChromeNotFound".to_owned(),
            })
            .await;

        // Then the dashboard shows the browser entry as Dead with the reason.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let (lifecycle, message, _) =
            dashboard_entry(&state, "web-fetch-browser").expect("entry should exist");
        assert_eq!(lifecycle, ActorLifecycle::Dead);
        assert_eq!(message.as_deref(), Some("ChromeNotFound"));
    }
}
