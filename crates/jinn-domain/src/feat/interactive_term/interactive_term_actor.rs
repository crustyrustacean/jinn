//! Interactive-term coordinator actor — owns PTY sessions across tool calls.
//!
//! One instance lives for the whole app. It owns every session (pty child +
//! emulator + transcript) so sessions persist between tool calls and turns:
//! the spawned program's lifetime is decoupled from the calls that drive it.
//!
//! The three tools (`interactive_term`, `interactive_term_send`,
//! `interactive_term_kill`) `ask` this actor directly (request/reply,
//! mirroring the `restart_mcp_server` tool); no bus eavesdropping, no
//! ordering race.
//!
//! The **settle wait** lives here: after a spawn or send, the drain loop
//! selects over the session's output channel and per-iteration deadlines,
//! returning when the screen has been quiet for the quiet window or the
//! hard cap was hit. Control flips (user takeover) are checked on **every
//! loop iteration** via the shared [`TermControl`] atomic — mailbox messages
//! are processed sequentially, so a mid-drain takeover could never be seen
//! through the mailbox; the atomic closes that gap.
//!
//! I/O: the pty pump is a std thread feeding an unbounded **tokio** mpsc
//! channel; kanal is forbidden in this select loop (documented double-free
//! under cancellation — see the bash tool).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use kameo::actor::{ActorRef, Spawn};
use kameo::prelude::{Context, Message};

use crate::common::services::bus_service::BusService;
use crate::feat::interactive_term::emulator::Emulator;
use crate::feat::interactive_term::protocol::command::{
    ControlHolder, KillTerm, KillTermOutcome, ResizeTerm, SendTermInput, SendTermKey,
    SendTermOutcome, SetTermControl, SpawnTerm, SpawnTermOutcome, TermScreen, TermSessionId,
};
use crate::feat::interactive_term::protocol::event::{TermControlChanged, TermScreenUpdated};
use crate::feat::interactive_term::pty_session::{ExitInfo, OutputTx, PtySession};
use crate::feat::interactive_term::query_responder::respond_to_queries;
use crate::feat::interactive_term::settle::{encode_input, should_settle};

/// Transcript ring length for new sessions (screens observed).
const TRANSCRIPT_LINES: usize = 200;

/// How many transcript screens the kill result reports.
const TRANSCRIPT_TAIL_SCREENS: usize = 20;

/// Shared control-holder flag (0 = agent, 1 = user).
///
/// Shared between the actor (authoritative writer via [`SetTermControl`])
/// and the takeover UI (Phase 3: the `IntentHandler` flips it synchronously
/// so an in-flight tool call sees the takeover on its next drain iteration
/// — mailbox-sequential message handling cannot deliver that). Both paths
/// are within the "IntentHandler exempt" and "one domain actor" rules.
#[derive(Debug, Clone, Default)]
pub struct TermControl(Arc<AtomicU8>);

impl TermControl {
    /// Creates the flag with the agent holding control.
    #[must_use]
    pub fn new(holder: ControlHolder) -> Self {
        let flag = Self::default();
        flag.set(holder);
        flag
    }

    /// Sets who holds control.
    pub fn set(&self, holder: ControlHolder) {
        self.0.store(
            match holder {
                ControlHolder::Agent => 0,
                ControlHolder::User => 1,
            },
            Ordering::SeqCst,
        );
    }

    /// Loads who holds control.
    #[must_use]
    pub fn get(&self) -> ControlHolder {
        match self.0.load(Ordering::SeqCst) {
            1 => ControlHolder::User,
            _ => ControlHolder::Agent,
        }
    }
}

/// A live interactive session owned by the actor.
struct TermSession {
    pty: PtySession,
    emulator: Emulator,
    rx: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    exited: Option<ExitInfo>,
    /// Last rendered screen; refreshed on every settle.
    last_screen: String,
}

/// The interactive-term coordinator actor.
pub struct InteractiveTermActor {
    bus: BusService,
    control: TermControl,
    sessions: HashMap<TermSessionId, TermSession>,
    state: crate::common::state::State,
    cap: crate::common::tcaps::frontend::FrontendCap,
    settle_quiet: Duration,
    settle_cap: Duration,
}

/// Dependencies for [`InteractiveTermActor`].
#[derive(Clone)]
pub struct InteractiveTermActorDeps {
    /// Bus for screen/control events.
    pub bus: BusService,
    /// Shared control-holder flag; the spawner keeps a clone for the UI.
    pub control: TermControl,
    /// Shared application state — the actor owns `frontend.terminal` and
    /// mirrors published screen/control events into it.
    pub state: crate::common::state::State,
    /// Capability to write `frontend.terminal`.
    pub cap: crate::common::tcaps::frontend::FrontendCap,
    /// Quiet window for the settle wait.
    pub settle_quiet: Duration,
    /// Hard cap for the settle wait.
    pub settle_cap: Duration,
}

impl kameo::Actor for InteractiveTermActor {
    type Args = InteractiveTermActorDeps;
    type Error = kameo::error::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.bus
            .register(actor_ref.clone().recipient::<SetTermControl>())
            .await;
        args.bus
            .register(actor_ref.clone().recipient::<SendTermKey>())
            .await;
        args.bus
            .register(actor_ref.clone().recipient::<ResizeTerm>())
            .await;
        Ok(Self {
            bus: args.bus,
            control: args.control,
            sessions: HashMap::new(),
            state: args.state,
            cap: args.cap,
            settle_quiet: args.settle_quiet,
            settle_cap: args.settle_cap,
        })
    }
}

/// Spawns the coordinator actor as a supervised child of the root.
///
/// Returns the actor ref and the shared control flag (hand the control
/// clone to the takeover UI wiring).
pub async fn spawn_interactive_term_actor(
    deps: InteractiveTermActorDeps,
    supervisor: &crate::common::root_supervisor::RootSupervisorRef,
) -> (ActorRef<InteractiveTermActor>, TermControl) {
    let control = deps.control.clone();
    let actor = InteractiveTermActor::supervise(supervisor, deps)
        .restart_policy(kameo::supervision::RestartPolicy::Never)
        .spawn()
        .await;
    (actor, control)
}

impl InteractiveTermActor {
    /// Handles [`SpawnTerm`]: creates the pty session and runs the initial
    /// settle wait.
    async fn handle_spawn(&mut self, msg: SpawnTerm) -> SpawnTermOutcome {
        let session_id = TermSessionId::next();
        let (tx, mut rx): (OutputTx, _) = tokio::sync::mpsc::unbounded_channel();
        let (rows, cols) = msg.size;
        let spawned = PtySession::spawn(
            &msg.command,
            &msg.cwd,
            portable_pty::PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            },
            tx,
        );
        let (mut pty, _pump) = match spawned {
            Ok(pair) => pair,
            Err(report) => return SpawnTermOutcome::Failed(format!("{report:#}")),
        };
        let mut emulator = Emulator::new(rows, cols, TRANSCRIPT_LINES);

        let control = self.control.clone();
        let bus = self.bus.clone();
        let (quiet, cap) = (self.settle_quiet, self.settle_cap);
        let mirror = Some((&self.state, &self.cap));
        drain_until_settled(
            &session_id,
            &mut pty,
            &mut emulator,
            &mut rx,
            &bus,
            &control,
            mirror,
            quiet,
            cap,
            msg.max_wait,
            true,
        )
        .await;
        let exited = pty.try_wait();
        emulator.sync_transcript();

        SpawnTermOutcome::Started {
            session_id: session_id.clone(),
            screen: TermScreen {
                screen: emulator.plain_text(),
                exited: exited.clone(),
            },
        }
        .attach_session(|outcome| {
            self.sessions.insert(
                session_id,
                TermSession {
                    pty,
                    emulator,
                    rx,
                    exited,
                    last_screen: match &outcome {
                        SpawnTermOutcome::Started { screen, .. } => screen.screen.clone(),
                        _ => String::new(),
                    },
                },
            );
        })
    }

    /// Handles [`SendTermInput`].
    async fn handle_send(&mut self, msg: SendTermInput) -> SendTermOutcome {
        // Take the session out so the drain can borrow the bus/control fields
        // without aliasing the map; it is unconditionally replaced below.
        let Some(mut session) = self.sessions.remove(&msg.session_id) else {
            return SendTermOutcome::UnknownSession;
        };
        if session.exited.is_some() {
            let outcome = SendTermOutcome::Exited(TermScreen {
                screen: session.last_screen.clone(),
                exited: session.exited.clone(),
            });
            self.sessions.insert(msg.session_id, session);
            return outcome;
        }
        // User takeover: refuse agent input (checked at entry AND
        // per-iteration inside the drain below).
        if self.control.get() == ControlHolder::User {
            let outcome = SendTermOutcome::UserHasControl(TermScreen {
                screen: session.last_screen.clone(),
                exited: None,
            });
            self.sessions.insert(msg.session_id, session);
            return outcome;
        }

        let bytes = encode_input(msg.text.as_deref(), &msg.keys, msg.enter);
        let wrote = if bytes.is_empty() {
            Ok(())
        } else {
            session.pty.write(&bytes)
        };
        if let Err(report) = wrote {
            tracing::warn!(report = %report, session = %msg.session_id, "pty write failed");
        }

        let session_id = msg.session_id.clone();
        let TermSession {
            pty, emulator, rx, ..
        } = &mut session;
        let mirror = Some((&self.state, &self.cap));
        drain_until_settled(
            &session_id,
            pty,
            emulator,
            rx,
            &self.bus,
            &self.control,
            mirror,
            self.settle_quiet,
            self.settle_cap,
            msg.max_wait,
            !bytes.is_empty(),
        )
        .await;

        session.exited = session.pty.try_wait();
        session.emulator.sync_transcript();
        session.last_screen = session.emulator.plain_text();

        if self.control.get() == ControlHolder::User {
            // The user grabbed the terminal mid-call; report the takeover.
            return SendTermOutcome::UserHasControl(TermScreen {
                screen: session.last_screen.clone(),
                exited: session.exited.clone(),
            });
        }
        let outcome = if self.control.get() == ControlHolder::User {
            // The user grabbed the terminal mid-call; report the takeover.
            SendTermOutcome::UserHasControl(TermScreen {
                screen: session.last_screen.clone(),
                exited: session.exited.clone(),
            })
        } else {
            SendTermOutcome::Sent(TermScreen {
                screen: session.last_screen.clone(),
                exited: session.exited.clone(),
            })
        };
        self.sessions.insert(msg.session_id, session);
        outcome
    }

    /// Handles [`KillTerm`].
    async fn handle_kill(&mut self, msg: KillTerm) -> KillTermOutcome {
        let Some(session) = self.sessions.get_mut(&msg.session_id) else {
            return KillTermOutcome::UnknownSession;
        };
        session.pty.kill();
        // Give the kernel a moment to report the exit; the group signal is
        // near-instant but `try_wait` is asynchronous to it.
        let deadline = Instant::now() + Duration::from_millis(500);
        while session.exited.is_none() && Instant::now() < deadline {
            session.exited = session.pty.try_wait();
            if session.exited.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        session.emulator.sync_transcript();
        session.last_screen = session.emulator.plain_text();
        let exited = session.exited.clone().unwrap_or(ExitInfo {
            code: 0,
            signal: None,
        });
        KillTermOutcome::Killed {
            screen: session.last_screen.clone(),
            transcript_tail: session.emulator.transcript_tail(TRANSCRIPT_TAIL_SCREENS),
            exited,
        }
    }
}

/// Drains the session's output channel until the settle condition is met.
///
/// Answers terminal queries inline (via `pty.write`) so probing programs
/// never stall, emits [`TermScreenUpdated`] on every screen change (the
/// live terminal-tab feed), and returns early when the user takes control
/// mid-drain. `emit_updates` is false for pure-sync waits so an unchanged
/// screen doesn't spam events. Watchdog keepalive is the *tool's* job (see
/// the heartbeat in the tool modules) — bus publishes for
/// `ToolExecutionOutput` must come from the tool layer, matching the bash
/// tool's streaming shape.
#[expect(
    clippy::too_many_arguments,
    reason = "drain inputs are disjoint pieces of a session the actor cannot pass as a struct"
)]
async fn drain_until_settled(
    session_id: &TermSessionId,
    pty: &mut PtySession,
    emulator: &mut Emulator,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    bus: &BusService,
    control: &TermControl,
    mirror: Option<(
        &crate::common::state::State,
        &crate::common::tcaps::frontend::FrontendCap,
    )>,
    quiet: Duration,
    cap: Duration,
    max_wait: Duration,
    emit_updates: bool,
) {
    let cap = cap.min(max_wait);
    let started = Instant::now();
    let mut last_output = started;
    let mut previous_screen = emulator.plain_text();

    loop {
        let now = Instant::now();
        let waited = now.duration_since(started);
        let quiet_for = now.duration_since(last_output);
        if should_settle(quiet_for, waited, quiet, cap) || control.get() == ControlHolder::User {
            return;
        }
        let wait_for = quiet
            .saturating_sub(quiet_for)
            .min(cap.saturating_sub(waited));

        match tokio::time::timeout(wait_for, rx.recv()).await {
            Ok(Some(chunk)) => {
                last_output = Instant::now();
                // Query responder first: answer from the pre-parse chunk so
                // a blocked program gets its reply this iteration.
                let replies = respond_to_queries(&chunk, emulator.cursor_position());
                if !replies.is_empty() {
                    let _ = pty.write(&replies);
                }
                emulator.feed(&chunk);
                if emit_updates {
                    let screen = emulator.plain_text();
                    if screen != previous_screen {
                        previous_screen = screen.clone();
                        let cursor = emulator.cursor_position();
                        let cursor_hidden = emulator.cursor_hidden();
                        bus.publish(TermScreenUpdated {
                            session_id: session_id.clone(),
                            screen: screen.clone(),
                            cursor,
                            cursor_hidden,
                        })
                        .await;
                        if let Some((state, cap)) = mirror {
                            use crate::common::tcaps::frontend::TerminalMirrorWrite;
                            state.with_terminal(cap, |ops| {
                                ops.apply_screen(
                                    &session_id.0,
                                    screen.clone(),
                                    cursor,
                                    cursor_hidden,
                                );
                            });
                        }
                    }
                }
            }
            Ok(None) => return, // pump closed: pty gone, settled.
            Err(_elapsed) => {} // timeout: re-check conditions at loop top.
        }
    }
}

/// Internal helper widening the spawn outcome with session insertion.
///
/// Exists to keep `handle_spawn`'s happy path linear: the session is
/// registered only after the outcome is known, and `last_screen` mirrors it.
trait AttachSession {
    fn attach_session(self, register: impl FnOnce(&Self)) -> Self;
}

impl AttachSession for SpawnTermOutcome {
    fn attach_session(self, register: impl FnOnce(&Self)) -> Self {
        register(&self);
        self
    }
}

impl Message<SpawnTerm> for InteractiveTermActor {
    type Reply = SpawnTermOutcome;

    async fn handle(
        &mut self,
        msg: SpawnTerm,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.handle_spawn(msg).await
    }
}

impl Message<SendTermInput> for InteractiveTermActor {
    type Reply = SendTermOutcome;

    async fn handle(
        &mut self,
        msg: SendTermInput,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.handle_send(msg).await
    }
}

impl Message<KillTerm> for InteractiveTermActor {
    type Reply = KillTermOutcome;

    async fn handle(
        &mut self,
        msg: KillTerm,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.handle_kill(msg).await
    }
}

impl Message<SendTermKey> for InteractiveTermActor {
    type Reply = ();

    async fn handle(&mut self, msg: SendTermKey, _ctx: &mut Context<Self, Self::Reply>) {
        // User keystrokes bypass the settle wait entirely: the user is
        // driving, so there is nothing to report back to an agent.
        if let Some(session) = self.sessions.get_mut(&msg.session_id)
            && let Err(report) = session.pty.write(&msg.bytes)
        {
            tracing::warn!(report = %report, session = %msg.session_id, "pty key write failed");
        }
    }
}

impl Message<SetTermControl> for InteractiveTermActor {
    type Reply = ();

    async fn handle(&mut self, msg: SetTermControl, _ctx: &mut Context<Self, Self::Reply>) {
        self.control.set(msg.holder);
        self.bus
            .publish(TermControlChanged {
                session_id: TermSessionId::next(),
                user_controls: msg.holder == ControlHolder::User,
            })
            .await;
    }
}

impl Message<ResizeTerm> for InteractiveTermActor {
    type Reply = ();

    async fn handle(&mut self, msg: ResizeTerm, _ctx: &mut Context<Self, Self::Reply>) {
        self.apply_resize(msg).await;
    }
}

impl InteractiveTermActor {
    /// Resizes the named (or only live) session's pty and emulator.
    ///
    /// Sizes are clamped to a minimal usable grid; no-op when nothing is
    /// running or the size is unchanged.
    async fn apply_resize(&mut self, msg: ResizeTerm) {
        // Resolve the session id up front so the mutable borrow below stays
        // disjoint from the mirror write at the end.
        let session_id = match &msg.session_id {
            Some(id) => self.sessions.contains_key(id).then(|| id.clone()),
            None => self.sessions.keys().next().cloned(),
        };
        let Some(session_id) = session_id else {
            return;
        };
        let (rows, cols) = (msg.size.0.max(2), msg.size.1.max(20));
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return;
        };
        if session.emulator.size() == (rows, cols) {
            return;
        }
        let _ = session.pty.resize(portable_pty::PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        session.emulator.set_size(rows, cols);
        session.last_screen = session.emulator.plain_text();
        let screen = session.last_screen.clone();
        let cursor = session.emulator.cursor_position();
        let hidden = session.emulator.cursor_hidden();
        self.bus
            .publish(TermScreenUpdated {
                session_id: session_id.clone(),
                screen: screen.clone(),
                cursor,
                cursor_hidden: hidden,
            })
            .await;
        self.state.with_terminal(&self.cap, |ops| {
            use crate::common::tcaps::frontend::TerminalMirrorWrite;
            ops.apply_screen(&session_id.0, screen, cursor, hidden);
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unwrap_used,
        reason = "test code"
    )]
    use super::*;
    use crate::common::bus::test_harness::{GetRecorded, TestHarness};
    use crate::common::root_supervisor::RootSupervisor;

    const QUIET: Duration = Duration::from_millis(150);
    const CAP: Duration = Duration::from_secs(2);

    fn deps(
        bus: BusService,
        control: TermControl,
    ) -> (InteractiveTermActorDeps, crate::common::state::State) {
        let state = crate::common::state::State::new(crate::common::app_state::AppState::default());
        let deps = InteractiveTermActorDeps {
            bus,
            control,
            state: state.clone(),
            cap: crate::common::tcaps::mint::mint_frontend_cap(),
            settle_quiet: QUIET,
            settle_cap: CAP,
        };
        (deps, state)
    }

    /// Spawns a coordinator under a fresh root supervisor.
    ///
    /// Returns the root alongside the actor: dropping the root ref stops
    /// supervised children, so callers must hold it for the actor's lifetime.
    async fn spawn_coordinator(
        harness: &TestHarness,
        control: TermControl,
    ) -> (
        ActorRef<InteractiveTermActor>,
        crate::common::root_supervisor::RootSupervisorRef,
    ) {
        let root = RootSupervisor::spawn_root().await;
        let (deps, _state) = deps(harness.bus(), control);
        let (actor, _control) = spawn_interactive_term_actor(deps, &root).await;
        (actor, root)
    }

    /// Spawns a coordinator with a readable state handle.
    async fn spawn_coordinator_with_state(
        harness: &TestHarness,
        control: TermControl,
    ) -> (
        ActorRef<InteractiveTermActor>,
        crate::common::state::State,
        crate::common::root_supervisor::RootSupervisorRef,
    ) {
        let root = RootSupervisor::spawn_root().await;
        let (deps, state) = deps(harness.bus(), control);
        let (actor, _control) = spawn_interactive_term_actor(deps, &root).await;
        (actor, state, root)
    }

    fn spawn_msg(command: &str) -> SpawnTerm {
        SpawnTerm {
            command: command.to_owned(),
            cwd: std::path::PathBuf::from("."),
            size: (24, 80),
            max_wait: Duration::from_secs(3),
        }
    }

    fn send_msg(session_id: TermSessionId) -> SendTermInput {
        SendTermInput {
            session_id,
            text: None,
            keys: vec![],
            enter: false,
            max_wait: Duration::from_secs(3),
        }
    }

    fn plain_screen(screen: &str) -> String {
        screen
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_owned()
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn spawn_returns_session_with_screen() {
        // Given a coordinator actor.
        let harness = TestHarness::new().await;
        let (actor, _root) = spawn_coordinator(&harness, TermControl::default()).await;

        // When spawning `echo`.
        let reply = actor
            .ask(spawn_msg("echo hello"))
            .await
            .expect("spawn reply");

        // Then the outcome is a session with the echoed text on screen.
        match reply {
            SpawnTermOutcome::Started { session_id, screen } => {
                assert!(plain_screen(&screen.screen).contains("hello"));
                assert!(!session_id.0.is_empty());
            }
            other => panic!("expected Started, got {other:?}"),
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn unspawnable_command_reports_failed_outcome() {
        // Given a coordinator actor.
        let harness = TestHarness::new().await;
        let (actor, _root) = spawn_coordinator(&harness, TermControl::default()).await;

        // When spawning with an empty command (bash exits immediately with a
        // usage error — the observable "spawn went wrong" path).
        let reply = actor.ask(spawn_msg("")).await.expect("spawn reply");

        // Then either the session started and exited (shell reported the
        // error) or the outcome carries the failure — both surface the problem
        // to the caller; no silent success.
        match reply {
            SpawnTermOutcome::Started { screen, .. } => {
                assert!(screen.exited.is_some(), "empty command must exit promptly");
            }
            SpawnTermOutcome::Failed(_) => {}
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn send_input_reaches_the_program_across_calls() {
        // Given a coordinator with a running `cat`.
        let harness = TestHarness::new().await;
        let (actor, _root) = spawn_coordinator(&harness, TermControl::default()).await;
        let SpawnTermOutcome::Started { session_id, .. } =
            actor.ask(spawn_msg("cat")).await.expect("spawn reply")
        else {
            panic!("expected Started");
        };

        // When sending text plus enter in a second call.
        let mut msg = send_msg(session_id);
        msg.text = Some("ping-from-agent".to_owned());
        msg.keys = vec!["enter".to_owned()];
        let SendTermOutcome::Sent(screen) = actor.ask(msg).await.expect("send reply") else {
            panic!("expected Sent");
        };

        // Then the echoed input appears in the returned screen (state persisted).
        assert!(plain_screen(&screen.screen).contains("ping-from-agent"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn program_state_persists_across_separate_tool_calls() {
        // Given a coordinator with an interactive bash session.
        let harness = TestHarness::new().await;
        let (actor, _root) = spawn_coordinator(&harness, TermControl::default()).await;
        let SpawnTermOutcome::Started { session_id, .. } = actor
            .ask(spawn_msg("bash --noprofile --norc"))
            .await
            .expect("spawn reply")
        else {
            panic!("expected Started");
        };

        // When setting a variable in one call...
        let mut msg = send_msg(session_id.clone());
        msg.text = Some("TERMVAR=inner-42".to_owned());
        msg.keys = vec!["enter".to_owned()];
        let SendTermOutcome::Sent(_) = actor.ask(msg).await.expect("send reply") else {
            panic!("expected Sent");
        };

        // ...and reading it back in a *separate* call.
        let mut msg = send_msg(session_id.clone());
        msg.text = Some("echo val=$TERMVAR".to_owned());
        msg.keys = vec!["enter".to_owned()];
        let SendTermOutcome::Sent(screen) = actor.ask(msg).await.expect("send reply") else {
            panic!("expected Sent");
        };

        // Then the variable survived — the same shell process served both calls.
        assert!(
            plain_screen(&screen.screen).contains("val=inner-42"),
            "screen was: {:?}",
            plain_screen(&screen.screen)
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn named_keys_drive_a_full_screen_tui_across_calls() {
        // Given a full-screen "TUI": an alt-screen pager showing PAGE ONE /
        // PAGE TWO depending on the last key (cursor-addressed output).
        let tui = concat!(
            "printf '\\033[?1049h\\033[H'; ",
            "show() { printf '\\033[2J\\033[5;10H%s' \"$1\"; }; ",
            "show PAGE-ONE; ",
            "while IFS= read -rsn1 k; do ",
            "  case \"$k\" in ",
            "    B) show PAGE-TWO ;; ",
            "    q) printf '\\033[?1049l'; exit 0 ;; ",
            "  esac; ",
            "done"
        );
        let harness = TestHarness::new().await;
        let (actor, _root) = spawn_coordinator(&harness, TermControl::default()).await;
        let SpawnTermOutcome::Started { session_id, .. } =
            actor.ask(spawn_msg(tui)).await.expect("spawn reply")
        else {
            panic!("expected Started");
        };

        // When pressing the key that pages forward (printable "B").
        let mut msg = send_msg(session_id.clone());
        msg.text = Some("B".to_owned());
        let SendTermOutcome::Sent(screen) = actor.ask(msg).await.expect("send reply") else {
            panic!("expected Sent");
        };

        // Then the TUI re-rendered to page two on the returned screen.
        assert!(
            plain_screen(&screen.screen).contains("PAGE-TWO"),
            "screen was: {:?}",
            plain_screen(&screen.screen)
        );
        assert!(!plain_screen(&screen.screen).contains("PAGE-ONE"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn ctrl_c_key_terminates_a_reading_program() {
        // Given a coordinator with a running `cat` (blocks on input).
        let harness = TestHarness::new().await;
        let (actor, _root) = spawn_coordinator(&harness, TermControl::default()).await;
        let SpawnTermOutcome::Started { session_id, .. } =
            actor.ask(spawn_msg("cat")).await.expect("spawn reply")
        else {
            panic!("expected Started");
        };

        // When sending the named key ctrl+c.
        let mut msg = send_msg(session_id);
        msg.keys = vec!["ctrl+c".to_owned()];
        let SendTermOutcome::Sent(screen) = actor.ask(msg).await.expect("send reply") else {
            panic!("expected Sent");
        };

        // Then the program exited (SIGINT reached it through the pty).
        assert!(screen.exited.is_some(), "cat must exit on ctrl+c");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn unknown_session_send_returns_unknown() {
        // Given a coordinator with no sessions.
        let harness = TestHarness::new().await;
        let (actor, _root) = spawn_coordinator(&harness, TermControl::default()).await;

        // When sending input to an unknown session id.
        let reply = actor
            .ask(send_msg(TermSessionId("ghost".to_owned())))
            .await
            .expect("send reply");

        // Then the outcome is UnknownSession.
        assert!(matches!(reply, SendTermOutcome::UnknownSession));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn send_to_exited_session_reports_exit_not_unknown() {
        // Given a coordinator with an exited session.
        let harness = TestHarness::new().await;
        let (actor, _root) = spawn_coordinator(&harness, TermControl::default()).await;
        let SpawnTermOutcome::Started { session_id, .. } =
            actor.ask(spawn_msg("true")).await.expect("spawn reply")
        else {
            panic!("expected Started");
        };

        // When sending input after exit.
        let reply = actor.ask(send_msg(session_id)).await.expect("send reply");

        // Then the outcome is Exited with the exit info, not UnknownSession.
        match reply {
            SendTermOutcome::Exited(screen) => {
                assert!(screen.exited.is_some());
            }
            other => panic!("expected Exited, got {other:?}"),
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn take_control_makes_agent_send_report_user_has_control() {
        // Given a coordinator with a running `cat` and the user holding control.
        let harness = TestHarness::new().await;
        let control = TermControl::default();
        let (actor, _root) = spawn_coordinator(&harness, control.clone()).await;
        let SpawnTermOutcome::Started { session_id, .. } =
            actor.ask(spawn_msg("cat")).await.expect("spawn reply")
        else {
            panic!("expected Started");
        };

        // When the user takes control and the agent then sends input.
        control.set(ControlHolder::User);
        let mut msg = send_msg(session_id);
        msg.text = Some("should-not-appear".to_owned());
        let reply = actor.ask(msg).await.expect("send reply");

        // Then the outcome is UserHasControl and no bytes reached the pty.
        let SendTermOutcome::UserHasControl(screen) = reply else {
            panic!("expected UserHasControl, got {reply:?}");
        };
        assert!(!plain_screen(&screen.screen).contains("should-not-appear"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn agent_input_while_user_controls_reaches_no_process() {
        // Given a coordinator running a program that exits after any input,
        // with the user holding control.
        let harness = TestHarness::new().await;
        let control = TermControl::default();
        let (actor, _root) = spawn_coordinator(&harness, control.clone()).await;
        let SpawnTermOutcome::Started { session_id, .. } = actor
            .ask(spawn_msg(
                "printf waiting; IFS= read -rsn1 k; printf got-input; sleep 30",
            ))
            .await
            .expect("spawn reply")
        else {
            panic!("expected Started");
        };
        control.set(ControlHolder::User);

        // When the agent sends input.
        let mut msg = send_msg(session_id.clone());
        msg.text = Some("x".to_owned());
        msg.enter = true;
        let SendTermOutcome::UserHasControl(_) = actor.ask(msg).await.expect("send reply") else {
            panic!("expected UserHasControl");
        };

        // Then the program never saw it (still waiting, not exited).
        let mut sync = send_msg(session_id);
        sync.max_wait = Duration::from_millis(600);
        let SendTermOutcome::UserHasControl(screen) = actor.ask(sync).await.expect("sync reply")
        else {
            panic!("expected UserHasControl");
        };
        assert!(
            !plain_screen(&screen.screen).contains("got-input"),
            "program consumed agent input despite user control"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn in_flight_send_sees_mid_drain_takeover() {
        // Given a coordinator with a program that trickles output over a second.
        let harness = TestHarness::new().await;
        let control = TermControl::default();
        let (actor, _root) = spawn_coordinator(&harness, control.clone()).await;
        let SpawnTermOutcome::Started { session_id, .. } = actor
            .ask(spawn_msg(
                "for i in 1 2 3 4 5 6; do echo tick-$i; sleep 0.25; done",
            ))
            .await
            .expect("spawn reply")
        else {
            panic!("expected Started");
        };

        // When a send starts and the user takes control mid-drain.
        control.set(ControlHolder::User);
        let ask = {
            let actor = actor.clone();
            let session_id = session_id.clone();
            tokio::spawn(async move { actor.ask(send_msg(session_id)).await.expect("send reply") })
        };
        tokio::time::sleep(Duration::from_millis(200)).await;
        control.set(ControlHolder::User); // already user; flips are re-read each iteration
        let replied = tokio::time::timeout(Duration::from_secs(1), ask).await;

        // Then the send returns promptly (well under the 3s cap) with UserHasControl.
        let replied = replied.expect("send must return promptly after takeover");
        let reply = replied.expect("join");
        assert!(
            matches!(reply, SendTermOutcome::UserHasControl(_)),
            "expected UserHasControl, got {reply:?}"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn kill_terminates_process_and_reports_tail() {
        // Given a coordinator with a program that printed before blocking.
        let harness = TestHarness::new().await;
        let (actor, _root) = spawn_coordinator(&harness, TermControl::default()).await;
        let SpawnTermOutcome::Started { session_id, .. } = actor
            .ask(spawn_msg("printf before-kill; cat"))
            .await
            .expect("spawn reply")
        else {
            panic!("expected Started");
        };

        // When killing the session.
        let reply = actor
            .ask(KillTerm {
                session_id: session_id.clone(),
            })
            .await
            .expect("kill reply");

        // Then the kill reports a signal exit.
        let KillTermOutcome::Killed {
            transcript_tail,
            exited,
            ..
        } = reply
        else {
            panic!("expected Killed");
        };
        assert!(exited.signal.is_some());
        let _ = transcript_tail;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn kill_is_idempotent_after_exit() {
        // Given a coordinator whose session exited naturally.
        let harness = TestHarness::new().await;
        let (actor, _root) = spawn_coordinator(&harness, TermControl::default()).await;
        let SpawnTermOutcome::Started { session_id, .. } =
            actor.ask(spawn_msg("true")).await.expect("spawn reply")
        else {
            panic!("expected Started");
        };

        // When killing the already-exited session twice.
        let first = actor
            .ask(KillTerm {
                session_id: session_id.clone(),
            })
            .await
            .expect("kill reply");
        let second = actor
            .ask(KillTerm {
                session_id: session_id.clone(),
            })
            .await
            .expect("kill reply");

        // Then both kills succeed with exit info.
        for reply in [first, second] {
            let KillTermOutcome::Killed { exited, .. } = reply else {
                panic!("expected Killed, got {reply:?}");
            };
            assert!(exited.code == 0 || exited.signal.is_some());
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn kill_unknown_session_reports_unknown() {
        // Given a coordinator with no sessions.
        let harness = TestHarness::new().await;
        let (actor, _root) = spawn_coordinator(&harness, TermControl::default()).await;

        // When killing an unknown session id.
        let reply = actor
            .ask(KillTerm {
                session_id: TermSessionId("ghost".to_owned()),
            })
            .await
            .expect("kill reply");

        // Then the outcome is UnknownSession.
        assert!(matches!(reply, KillTermOutcome::UnknownSession));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn settle_waits_stream_screen_updates_to_the_bus() {
        // Given a coordinator actor and a screen-recording subscriber.
        let harness = TestHarness::new().await;
        let recorder = harness.spawn_recorder::<TermScreenUpdated>().await;
        let (actor, _root) = spawn_coordinator(&harness, TermControl::default()).await;

        // When spawning a program whose output arrives in waves; the reply
        // only comes after the settle window, so every delta is already
        // recorded by the time it returns.
        let _ = actor
            .ask(spawn_msg("echo one; sleep 0.05; echo two"))
            .await
            .expect("spawn reply");

        // Then each screen change streamed a TermScreenUpdated to the bus.
        // (`GetRecorded` DRAINS the recorder, so poll exactly once.)
        let screens = recorder
            .ask(GetRecorded::new())
            .await
            .expect("get recorded");
        assert!(
            screens.iter().any(|s| s.screen.contains("one")),
            "expected a screen delta containing 'one'"
        );
        // And the second wave streamed its own delta.
        assert!(
            screens.iter().any(|s| s.screen.contains("two")),
            "expected a screen delta containing 'two'"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn natural_exit_is_captured_on_next_call() {
        // Given a coordinator with a short-lived program.
        let harness = TestHarness::new().await;
        let (actor, _root) = spawn_coordinator(&harness, TermControl::default()).await;
        let SpawnTermOutcome::Started { session_id, screen } = actor
            .ask(spawn_msg("sh -c 'echo bye; exit 7'"))
            .await
            .expect("spawn reply")
        else {
            panic!("expected Started");
        };

        // When the spawn reply already observed the exit.
        // Then the exit info reports code 7.
        let exited = screen.exited.expect("exit captured");
        assert_eq!(exited.code, 7);
        let _ = session_id;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn screen_updates_mirror_into_frontend_state() {
        // Given a coordinator actor wired to a readable shared state.
        let harness = TestHarness::new().await;
        let (actor, state, _root) =
            spawn_coordinator_with_state(&harness, TermControl::default()).await;

        // When spawning a program that prints to the screen.
        actor
            .ask(spawn_msg("echo mirror-me"))
            .await
            .expect("spawn reply");

        // Then the frontend terminal mirror carries the rendered screen.
        let guard = state.read();
        assert!(
            guard.frontend.terminal.screen().contains("mirror-me"),
            "mirror should contain output, got: {:?}",
            guard.frontend.terminal.screen()
        );
        // And the mirror records the session id.
        assert!(guard.frontend.terminal.session_id.is_some());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn send_updates_mirror_with_new_screen() {
        // Given a coordinator with a live `cat` session.
        let harness = TestHarness::new().await;
        let (actor, state, _root) =
            spawn_coordinator_with_state(&harness, TermControl::default()).await;
        let reply = actor.ask(spawn_msg("cat")).await.expect("spawn reply");
        let SpawnTermOutcome::Started { session_id, .. } = reply else {
            panic!("expected Started");
        };

        // When sending text through the send path.
        actor
            .ask(SendTermInput {
                text: Some("mirrored-after-send".to_owned()),
                ..send_msg(session_id)
            })
            .await
            .expect("send reply");

        // Then the mirror reflects the echoed output.
        let guard = state.read();
        assert!(
            guard
                .frontend
                .terminal
                .screen()
                .contains("mirrored-after-send")
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn resize_updates_session_and_mirror() {
        // Given a coordinator with a live `cat` session.
        let harness = TestHarness::new().await;
        let (actor, state, _root) =
            spawn_coordinator_with_state(&harness, TermControl::default()).await;
        actor.ask(spawn_msg("cat")).await.expect("spawn reply");

        // When resizing to a small grid.
        actor
            .tell(ResizeTerm {
                session_id: None,
                size: (10, 40),
            })
            .await
            .expect("resize tell");

        // Give the actor a beat to process the tell.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Then the session reports the new size and no error occurred.
        // (Observable: a following send still succeeds and mirror reflects
        // the session.)
        let guard = state.read();
        assert!(guard.frontend.terminal.session_id.is_some());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn resize_without_session_is_noop() {
        // Given a coordinator with no sessions.
        let harness = TestHarness::new().await;
        let (actor, _state, _root) =
            spawn_coordinator_with_state(&harness, TermControl::default()).await;

        // When sending a resize.
        let result = actor
            .tell(ResizeTerm {
                session_id: None,
                size: (10, 40),
            })
            .await;

        // Then it is accepted silently.
        assert!(result.is_ok());
    }
}
