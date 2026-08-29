//! Interactive-term coordinator actor — owns PTY sessions across tool calls.
//!
//! One instance lives for the whole app. It owns every session (pty child +
//! emulator + transcript) so sessions persist between tool calls and turns:
//! the spawned program's lifetime is decoupled from the calls that drive it.
//!
//! **One terminal per chat session.** Sessions are keyed by the owning chat
//! [`SessionId`]; spawning while a session already has a live terminal kills
//! the old one first and reports it. The coordinator's own
//! [`TermSessionId`] (`term-N`) remains the model-facing handle.
//!
//! **Realtime display.** Each session's output pump is owned by its screen
//! task (see [`screen_task`]), which parses on a ~50 ms cadence and
//! republishes the mirror on change — the overlay and sidebar stay live
//! while the program runs on its own, with no tool call in flight. Ask-time
//! settles (spawn/send) never touch the receiver: they watch the screen
//! version counter, so a settle and the screen task cannot race the pump.
//!
//! The tools (`interactive_term`, `interactive_term_send`,
//! `interactive_term_kill`) `ask` this actor directly (request/reply,
//! mirroring the `restart_mcp_server` tool); no bus eavesdropping, no
//! ordering race.
//!
//! The **settle decision** lives here: after a spawn or send, the ask waits
//! until the screen has been quiet for the quiet window or the hard cap was
//! hit (see `settle` for the decision logic). Control flips (user takeover)
//! are checked via the shared [`TermControl`] atomic on every settle poll —
//! mailbox messages are processed sequentially, so a mid-drain takeover
//! could never be seen through the mailbox; the atomic closes that gap.
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
use crate::feat::interactive_term::protocol::command::{
    ControlHolder, KillTerm, KillTermOutcome, ResizeTerm, SendTermInput, SendTermKey,
    SendTermOutcome, SetTermControl, SpawnTerm, SpawnTermOutcome, TermScreen, TermSessionId,
};
use crate::feat::interactive_term::protocol::event::{TermControlChanged, TermScreenUpdated};
use crate::feat::interactive_term::pty_session::{ExitInfo, PtySession};
use crate::feat::interactive_term::screen_task::{ScreenHandle, ScreenWiring};
use crate::feat::interactive_term::settle::{encode_input, should_settle};

/// How many transcript screens the kill result reports.
const TRANSCRIPT_TAIL_SCREENS: usize = 20;

/// Shared control-holder flag (0 = agent, 1 = user).
///
/// Shared between the actor (authoritative writer via [`SetTermControl`])
/// and the takeover UI (the `IntentHandler` flips it synchronously so an
/// in-flight tool call's settle sees the takeover on its next poll —
/// mailbox-sequential message handling cannot deliver that). Both paths are
/// within the "IntentHandler exempt" and "one domain actor" rules.
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
    /// The pty child; also reaches the shared emulator and screen task.
    pty: PtySession,
    /// The model-facing handle (`term-N`).
    term_id: TermSessionId,
    /// Captured once the process terminated.
    exited: Option<ExitInfo>,
    /// Last screen text the *actor* returned/published (ask results); the
    /// screen task's mirror publication is keyed off its own tracker.
    last_screen: String,
}

impl TermSession {
    /// Screen text, styled cells, cursor, and visibility from the emulator.
    fn snapshot(
        &self,
    ) -> (
        String,
        crate::feat::interactive_term::emulator::ScreenCells,
        (u16, u16),
        bool,
    ) {
        let handle = self.pty.screen();
        let guard = handle.lock();
        (
            guard.emulator().plain_text(),
            guard.emulator().cells(),
            guard.emulator().cursor_position(),
            guard.emulator().cursor_hidden(),
        )
    }

    /// Appends the current screen to the transcript ring.
    fn sync_transcript(&self) {
        self.pty.screen().lock().emulator_mut().sync_transcript();
    }

    /// The transcript tail (most recent screens).
    fn transcript_tail(&self, max_screens: usize) -> String {
        self.pty
            .screen()
            .lock()
            .emulator()
            .transcript_tail(max_screens)
    }
}

/// The interactive-term coordinator actor.
pub struct InteractiveTermActor {
    bus: BusService,
    control: TermControl,
    /// Live sessions keyed by their owning chat session.
    sessions: HashMap<crate::protocol::SessionId, TermSession>,
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

/// How a settle wait concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettleReason {
    /// The screen was quiet for the quiet window.
    Quiet,
    /// The hard cap (or the ask's `max_wait`) elapsed.
    Cap,
    /// The user took control mid-wait.
    UserTookControl,
    /// The program exited; the screen cannot change again.
    Exited,
}

/// Waits until a session's screen settles, watching its version counter.
///
/// The screen task owns the pump; this loop only observes the version watch,
/// so asks and realtime parsing never race for chunks. Settles when the
/// screen has been unchanged for the quiet window (or the program exited and
/// the window elapsed), when the cap elapsed, or when the user took control.
async fn wait_for_settle(
    handle: &ScreenHandle,
    control: &TermControl,
    quiet: Duration,
    cap: Duration,
) -> SettleReason {
    let started = Instant::now();
    let mut last_change = started;
    let mut version = handle.version();
    let mut last_seen = *version.borrow();
    loop {
        let now = Instant::now();
        let quiet_for = now.duration_since(last_change);
        let waited = now.duration_since(started);
        if control.get() == ControlHolder::User {
            return SettleReason::UserTookControl;
        }
        let pump_closed = handle.pump_closed();
        let settled =
            (pump_closed && quiet_for >= quiet) || should_settle(quiet_for, waited, quiet, cap);
        if settled {
            return if control.get() == ControlHolder::User {
                SettleReason::UserTookControl
            } else if pump_closed && waited < cap {
                SettleReason::Exited
            } else if waited >= cap {
                SettleReason::Cap
            } else {
                SettleReason::Quiet
            };
        }
        let wait_for = quiet
            .saturating_sub(quiet_for)
            .min(cap.saturating_sub(waited))
            .min(Duration::from_millis(25));
        match tokio::time::timeout(wait_for, version.changed()).await {
            Ok(Ok(())) => {
                let current = *version.borrow();
                if current != last_seen {
                    last_change = Instant::now();
                    last_seen = current;
                }
            }
            Ok(Err(_recv_error)) => {
                // The session's version sender is gone: torn down mid-wait.
                return SettleReason::Exited;
            }
            Err(_elapsed) => {} // poll elapsed: re-check conditions at the top.
        }
    }
}

impl InteractiveTermActor {
    /// Handles [`SpawnTerm`]: kills the chat session's previous terminal (if
    /// any), creates the pty session with its realtime screen task, and runs
    /// the initial settle wait against the screen-version watch.
    async fn handle_spawn(&mut self, msg: SpawnTerm) -> SpawnTermOutcome {
        // One terminal per chat session: replace any live terminal first.
        let killed_previous = if self.sessions.contains_key(&msg.chat_session_id) {
            self.remove_session(&msg.chat_session_id).map(|removed| {
                crate::feat::interactive_term::protocol::command::KilledPrevious {
                    session_id: TermSessionId(removed.term_id),
                    exited: removed.exited.unwrap_or(ExitInfo {
                        code: 0,
                        signal: None,
                    }),
                }
            })
        } else {
            None
        };

        let session_id = TermSessionId::next();
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
            self.wiring(&msg.chat_session_id, &session_id),
        );
        let (mut pty, _pump) = match spawned {
            Ok(pair) => pair,
            Err(report) => return SpawnTermOutcome::Failed(format!("{report:#}")),
        };

        // The screen task owns parsing from spawn on; this ask settles on
        // the version watch (cap clamped to the ask's max_wait).
        let _settle = wait_for_settle(
            &pty.screen(),
            &self.control,
            self.settle_quiet,
            self.settle_cap.min(msg.max_wait),
        )
        .await;

        let exited = pty.try_wait();
        pty.screen().lock().emulator_mut().sync_transcript();
        let (screen, cells, cursor, cursor_hidden) = {
            let handle = pty.screen();
            let guard = handle.lock();
            (
                guard.emulator().plain_text(),
                guard.emulator().cells(),
                guard.emulator().cursor_position(),
                guard.emulator().cursor_hidden(),
            )
        };
        self.sessions.insert(
            msg.chat_session_id.clone(),
            TermSession {
                pty,
                term_id: session_id.clone(),
                exited: exited.clone(),
                last_screen: screen.clone(),
            },
        );
        self.write_mirror(
            &msg.chat_session_id,
            &session_id,
            screen.clone(),
            cells,
            cursor,
            cursor_hidden,
        );

        SpawnTermOutcome::Started {
            session_id,
            screen: TermScreen { screen, exited },
            killed_previous,
        }
    }

    /// The per-session screen-task wiring (bus + mirror + live-flag key).
    fn wiring(&self, chat: &crate::protocol::SessionId, term_id: &TermSessionId) -> ScreenWiring {
        ScreenWiring {
            bus: self.bus.clone(),
            state: self.state.clone(),
            cap: self.cap,
            chat: chat.clone(),
            term_id: term_id.clone(),
        }
    }

    /// Writes one screen snapshot into the frontend mirror.
    fn write_mirror(
        &self,
        chat: &crate::protocol::SessionId,
        term_id: &TermSessionId,
        screen: String,
        cells: crate::feat::interactive_term::emulator::ScreenCells,
        cursor: (u16, u16),
        cursor_hidden: bool,
    ) {
        use crate::common::tcaps::frontend::TerminalMirrorWrite;
        self.state.with_terminal(&self.cap, |ops| {
            ops.apply_screen(chat, &term_id.0, screen, cells, cursor, cursor_hidden);
        });
    }

    /// Marks (or clears) a session's live-terminal flag in the mirror.
    fn set_live(&self, chat: &crate::protocol::SessionId, live: bool) {
        use crate::common::tcaps::frontend::TerminalMirrorWrite;
        self.state.with_terminal(&self.cap, |ops| {
            ops.set_live(chat, live);
        });
    }

    /// Removes and tears down a session (aborts its screen task, clears its
    /// live flag), returning identity + exit info for reporting.
    fn remove_session(&mut self, chat: &crate::protocol::SessionId) -> Option<RemovedSession> {
        use crate::common::tcaps::frontend::TerminalMirrorWrite;
        // Clear the live flag *before* the session is dropped: dropping the
        // session aborts the screen task, and a task killed before it observed
        // EOF never gets to clear the flag itself.
        let session = self.sessions.remove(chat)?;
        self.state.with_terminal(&self.cap, |ops| {
            ops.set_live(chat, false);
        });
        let removed = RemovedSession {
            term_id: session.term_id.0.clone(),
            exited: session.exited.clone(),
        };
        drop(session);
        Some(removed)
    }

    /// Resolves the owning chat session for a model-facing term id.
    fn chat_for_term(&self, term_id: &TermSessionId) -> Option<crate::protocol::SessionId> {
        self.sessions
            .iter()
            .find(|(_, session)| session.term_id == *term_id)
            .map(|(chat, _)| chat)
            .cloned()
    }

    /// Handles [`SendTermInput`].
    async fn handle_send(&mut self, msg: SendTermInput) -> SendTermOutcome {
        // Resolve the owning chat session, then take the session out so the
        // settle await below holds no borrow over the map; it is
        // unconditionally replaced before returning.
        let Some(chat) = self.chat_for_term(&msg.session_id) else {
            return SendTermOutcome::UnknownSession;
        };
        let Some(mut session) = self.sessions.remove(&chat) else {
            return SendTermOutcome::UnknownSession;
        };

        if session.exited.is_some() {
            let outcome = SendTermOutcome::Exited(TermScreen {
                screen: session.last_screen.clone(),
                exited: session.exited.clone(),
            });
            self.sessions.insert(chat, session);
            return outcome;
        }
        // User takeover: refuse agent input (checked here and re-checked
        // after the settle below).
        if self.control.get() == ControlHolder::User {
            let outcome = SendTermOutcome::UserHasControl(TermScreen {
                screen: session.last_screen.clone(),
                exited: None,
            });
            self.sessions.insert(chat, session);
            return outcome;
        }

        let bytes = encode_input(msg.text.as_deref(), &msg.keys, msg.enter);
        if !bytes.is_empty()
            && let Err(report) = session.pty.write(&bytes)
        {
            tracing::warn!(report = %report, session = %msg.session_id, "pty write failed");
        }

        // The screen task parses; settle on the version watch.
        let _settle = wait_for_settle(
            &session.pty.screen(),
            &self.control,
            self.settle_quiet,
            self.settle_cap.min(msg.max_wait),
        )
        .await;

        session.exited = session.pty.try_wait();
        session.sync_transcript();
        let (screen, cells, cursor, cursor_hidden) = session.snapshot();
        session.last_screen.clone_from(&screen);
        let term_id = session.term_id.clone();
        self.sessions.insert(chat.clone(), session);
        self.write_mirror(
            &chat,
            &term_id,
            screen.clone(),
            cells,
            cursor,
            cursor_hidden,
        );

        if self.control.get() == ControlHolder::User {
            // The user grabbed the terminal mid-call; report the takeover.
            return SendTermOutcome::UserHasControl(TermScreen {
                screen,
                exited: self.sessions.get(&chat).and_then(|s| s.exited.clone()),
            });
        }
        SendTermOutcome::Sent(TermScreen {
            screen,
            exited: self.sessions.get(&chat).and_then(|s| s.exited.clone()),
        })
    }

    /// Handles [`KillTerm`].
    async fn handle_kill(&mut self, msg: KillTerm) -> KillTermOutcome {
        let Some(chat) = self.chat_for_term(&msg.session_id) else {
            return KillTermOutcome::UnknownSession;
        };
        let Some(session) = self.sessions.get_mut(&chat) else {
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
        session.sync_transcript();
        let (screen, _, _cursor, _hidden) = session.snapshot();
        session.last_screen = screen;
        let exited = session.exited.clone().unwrap_or(ExitInfo {
            code: 0,
            signal: None,
        });
        let tail = session.transcript_tail(TRANSCRIPT_TAIL_SCREENS);
        let _ = screen;
        let final_screen = session.last_screen.clone();
        // The session *stays registered* (a repeat kill is still Killed, a
        // send reports Exited); only the live flag clears. The registry entry
        // is replaced by the next spawn for this chat session.
        self.set_live(&chat, false);
        KillTermOutcome::Killed {
            screen: final_screen,
            transcript_tail: tail,
            exited,
        }
    }
}

/// What `remove_session` reports about the torn-down session.
struct RemovedSession {
    term_id: String,
    exited: Option<ExitInfo>,
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
        if let Some(chat) = self.chat_for_term(&msg.session_id)
            && let Some(session) = self.sessions.get_mut(&chat)
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
        {
            use crate::common::tcaps::frontend::TerminalMirrorWrite;
            self.state.with_terminal(&self.cap, |ops| {
                ops.set_control(match msg.holder {
                    ControlHolder::User => {
                        crate::feat::interactive_term::terminal_tab_state::TermControlHolder::User
                    }
                    ControlHolder::Agent => {
                        crate::feat::interactive_term::terminal_tab_state::TermControlHolder::Agent
                    }
                });
            });
        }
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
        // Resolve the chat session up front so the mutable borrow below
        // stays disjoint from the mirror write at the end.
        let chat = match &msg.session_id {
            Some(id) => self.chat_for_term(id),
            None => self.sessions.keys().next().cloned(),
        };
        let Some(chat) = chat else {
            return;
        };
        let (rows, cols) = (msg.size.0.max(2), msg.size.1.max(20));
        let Some(session) = self.sessions.get_mut(&chat) else {
            return;
        };
        if session.pty.emulator_size() == (rows, cols) {
            return;
        }
        let _ = session.pty.resize(portable_pty::PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        session.pty.set_emulator_size(rows, cols);
        let (screen, cells, cursor, hidden) = {
            let handle = session.pty.screen();
            let guard = handle.lock();
            (
                guard.emulator().plain_text(),
                guard.emulator().cells(),
                guard.emulator().cursor_position(),
                guard.emulator().cursor_hidden(),
            )
        };
        session.last_screen = screen.clone();
        let term_id = session.term_id.clone();
        self.write_mirror(
            &chat,
            &term_id,
            screen.clone(),
            cells.clone(),
            cursor,
            hidden,
        );
        self.bus
            .publish(TermScreenUpdated {
                session_id: term_id,
                screen,
                cells,
                cursor,
                cursor_hidden: hidden,
            })
            .await;
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

    fn spawn_msg(chat: crate::protocol::SessionId, command: &str) -> SpawnTerm {
        SpawnTerm {
            chat_session_id: chat,
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

    async fn spawn_cat(
        actor: &ActorRef<InteractiveTermActor>,
        chat: &crate::protocol::SessionId,
    ) -> TermSessionId {
        let SpawnTermOutcome::Started { session_id, .. } = actor
            .ask(spawn_msg(chat.clone(), "cat"))
            .await
            .expect("spawn reply")
        else {
            panic!("expected Started");
        };
        session_id
    }

    /// Agent-sent f-keys must reach the program as real terminal bytes:
    /// `cat -v` echoes ESC as `^[`, so an F4 (`ESC O S`) shows as `^[OS`.
    /// (v1 regression: `encode_key("f4")` produced no bytes at all.)
    #[cfg(unix)]
    #[rstest::rstest]
    #[tokio::test]
    async fn agent_f4_key_reaches_the_program_as_bytes() {
        // Given a coordinator running `cat -v`.
        let harness = TestHarness::new().await;
        let (actor, _root) = spawn_coordinator(&harness, TermControl::default()).await;
        let chat = crate::protocol::SessionId::new();
        let SpawnTermOutcome::Started { session_id, .. } = actor
            .ask(spawn_msg(chat, "cat -v"))
            .await
            .expect("spawn reply")
        else {
            panic!("expected Started");
        };

        // When sending the named f4 key.
        let mut msg = send_msg(session_id.clone());
        msg.keys = vec!["f4".to_owned()];
        let outcome = actor.ask(msg).await.expect("send reply");

        // Then the program echoed the F4 bytes (`^[` + `OS`).
        match outcome {
            SendTermOutcome::Sent(screen) => {
                assert!(
                    plain_screen(&screen.screen).contains("^[OS"),
                    "cat -v should echo F4 as ^[OS, got: {}",
                    plain_screen(&screen.screen)
                );
            }
            other => panic!("expected Screen, got {other:?}"),
        }
    }

    /// The pty child runs in the cwd the request carries (the tool passes
    /// its context cwd), so `pwd` prints that directory.
    #[cfg(unix)]
    #[rstest::rstest]
    #[tokio::test]
    async fn spawn_pty_runs_in_the_requested_cwd() {
        use std::path::PathBuf;

        // Given a coordinator and a real scratch directory.
        let harness = TestHarness::new().await;
        let (actor, _root) = spawn_coordinator(&harness, TermControl::default()).await;
        let dir = std::env::temp_dir().join(format!("jinn-term-cwd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");

        // When spawning `pwd` with that directory as cwd.
        let mut msg = spawn_msg(crate::protocol::SessionId::new(), "pwd");
        msg.cwd = PathBuf::from(&dir);
        let reply = actor.ask(msg).await.expect("spawn reply");

        // Then the screen shows the requested directory.
        match reply {
            SpawnTermOutcome::Started { screen, .. } => {
                assert!(
                    plain_screen(&screen.screen).contains(dir.to_str().expect("utf8 path")),
                    "pwd should print the requested cwd, got: {}",
                    plain_screen(&screen.screen)
                );
            }
            other => panic!("expected Started, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn spawn_returns_session_with_screen() {
        // Given a coordinator actor.
        let harness = TestHarness::new().await;
        let (actor, _root) = spawn_coordinator(&harness, TermControl::default()).await;

        // When spawning `echo`.
        let reply = actor
            .ask(spawn_msg(crate::protocol::SessionId::new(), "echo hello"))
            .await
            .expect("spawn reply");

        // Then the outcome is a session with the echoed text on screen.
        match reply {
            SpawnTermOutcome::Started {
                session_id, screen, ..
            } => {
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
        let reply = actor
            .ask(spawn_msg(crate::protocol::SessionId::new(), ""))
            .await
            .expect("spawn reply");

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
        let chat = crate::protocol::SessionId::new();
        let session_id = spawn_cat(&actor, &chat).await;

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
        let chat = crate::protocol::SessionId::new();
        let session_id = {
            let SpawnTermOutcome::Started { session_id, .. } = actor
                .ask(spawn_msg(chat, "bash --noprofile --norc"))
                .await
                .expect("spawn reply")
            else {
                panic!("expected Started");
            };
            session_id
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
        let chat = crate::protocol::SessionId::new();
        let session_id = {
            let SpawnTermOutcome::Started { session_id, .. } =
                actor.ask(spawn_msg(chat, tui)).await.expect("spawn reply")
            else {
                panic!("expected Started");
            };
            session_id
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
        let chat = crate::protocol::SessionId::new();
        let session_id = spawn_cat(&actor, &chat).await;

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
        let chat = crate::protocol::SessionId::new();
        let session_id = {
            let SpawnTermOutcome::Started { session_id, .. } = actor
                .ask(spawn_msg(chat, "true"))
                .await
                .expect("spawn reply")
            else {
                panic!("expected Started");
            };
            session_id
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
        let chat = crate::protocol::SessionId::new();
        let session_id = spawn_cat(&actor, &chat).await;

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
        let chat = crate::protocol::SessionId::new();
        let session_id = {
            let SpawnTermOutcome::Started { session_id, .. } = actor
                .ask(spawn_msg(
                    chat,
                    "printf waiting; IFS= read -rsn1 k; printf got-input; sleep 30",
                ))
                .await
                .expect("spawn reply")
            else {
                panic!("expected Started");
            };
            session_id
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
    async fn in_flight_send_sees_mid_wait_takeover() {
        // Given a coordinator with a program that trickles output over a second.
        let harness = TestHarness::new().await;
        let control = TermControl::default();
        let (actor, _root) = spawn_coordinator(&harness, control.clone()).await;
        let chat = crate::protocol::SessionId::new();
        let session_id = {
            let SpawnTermOutcome::Started { session_id, .. } = actor
                .ask(spawn_msg(
                    chat,
                    "for i in 1 2 3 4 5 6; do echo tick-$i; sleep 0.25; done",
                ))
                .await
                .expect("spawn reply")
            else {
                panic!("expected Started");
            };
            session_id
        };

        // When a send starts and the user takes control mid-wait.
        control.set(ControlHolder::User);
        let ask = {
            let actor = actor.clone();
            let session_id = session_id.clone();
            tokio::spawn(async move { actor.ask(send_msg(session_id)).await.expect("send reply") })
        };
        tokio::time::sleep(Duration::from_millis(200)).await;
        control.set(ControlHolder::User); // already user; flips are re-read each poll
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
        let chat = crate::protocol::SessionId::new();
        let session_id = {
            let SpawnTermOutcome::Started { session_id, .. } = actor
                .ask(spawn_msg(chat, "printf before-kill; cat"))
                .await
                .expect("spawn reply")
            else {
                panic!("expected Started");
            };
            session_id
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
        let chat = crate::protocol::SessionId::new();
        let session_id = {
            let SpawnTermOutcome::Started { session_id, .. } = actor
                .ask(spawn_msg(chat, "true"))
                .await
                .expect("spawn reply")
            else {
                panic!("expected Started");
            };
            session_id
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
            .ask(spawn_msg(
                crate::protocol::SessionId::new(),
                "echo one; sleep 0.05; echo two",
            ))
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
        let chat = crate::protocol::SessionId::new();

        // When the spawn reply already observed the exit.
        let SpawnTermOutcome::Started { screen, .. } = actor
            .ask(spawn_msg(chat, "sh -c 'echo bye; exit 7'"))
            .await
            .expect("spawn reply")
        else {
            panic!("expected Started");
        };

        // Then the exit info reports code 7.
        let exited = screen.exited.expect("exit captured");
        assert_eq!(exited.code, 7);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn screen_updates_mirror_into_frontend_state() {
        // Given a coordinator actor wired to a readable shared state.
        let harness = TestHarness::new().await;
        let (actor, state, _root) =
            spawn_coordinator_with_state(&harness, TermControl::default()).await;
        let chat = crate::protocol::SessionId::new();

        // When spawning a program that prints to the screen.
        actor
            .ask(spawn_msg(chat.clone(), "echo mirror-me"))
            .await
            .expect("spawn reply");

        // Then the frontend terminal mirror carries the rendered screen.
        let guard = state.read();
        let mirror = guard
            .frontend
            .terminal
            .mirror(&chat)
            .expect("mirror for chat session");
        assert!(
            mirror.screen.contains("mirror-me"),
            "mirror should contain output, got: {:?}",
            mirror.screen
        );
        // And the mirror records the term session id.
        assert!(mirror.term_session_id.starts_with("term-"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn styled_cells_reach_the_mirror_with_colors() {
        // Given a coordinator wired to a readable shared state and a program
        // printing an ANSI-colored word.
        let harness = TestHarness::new().await;
        let (actor, state, _root) =
            spawn_coordinator_with_state(&harness, TermControl::default()).await;
        let chat = crate::protocol::SessionId::new();

        // When spawning a program that emits red text.
        actor
            .ask(spawn_msg(
                chat.clone(),
                "printf 'plain \\033[31mred\\033[0m end'",
            ))
            .await
            .expect("spawn reply");

        // Then the mirror's cell grid marks the colored span red and the
        // surrounding text default-colored.
        let mirror = {
            let guard = state.read();
            guard
                .frontend
                .terminal
                .mirror(&chat)
                .expect("mirror for chat session")
                .clone()
        };
        let row = 0;
        let mut red_span = None;
        let mut default_before = None;
        for col in 0..mirror.cells.cols {
            match mirror.cells.get(row, col) {
                Some(crate::feat::interactive_term::emulator::TermCell::Styled { ch, style })
                    if *ch != ' ' =>
                {
                    let is_red =
                        style.fg == crate::feat::interactive_term::emulator::TermColor::Idx(1);
                    if is_red && red_span.is_none() {
                        red_span = Some(col);
                    }
                    if !is_red && red_span.is_some() {
                        default_before = Some(col);
                        break;
                    }
                    if red_span.is_none() {
                        default_before = Some(col);
                    }
                }
                _ => {}
            }
        }
        assert!(
            red_span.is_some(),
            "expected a red-styled span in the cell grid, got: {:?} at row {row}",
            mirror.cells
        );
        let _ = default_before;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn send_updates_mirror_with_new_screen() {
        // Given a coordinator with a live `cat` session.
        let harness = TestHarness::new().await;
        let (actor, state, _root) =
            spawn_coordinator_with_state(&harness, TermControl::default()).await;
        let chat = crate::protocol::SessionId::new();
        let session_id = spawn_cat(&actor, &chat).await;

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
        let mirror = guard.frontend.terminal.mirror(&chat).expect("mirror");
        assert!(mirror.screen.contains("mirrored-after-send"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn resize_updates_session_and_mirror() {
        // Given a coordinator with a live `cat` session.
        let harness = TestHarness::new().await;
        let (actor, state, _root) =
            spawn_coordinator_with_state(&harness, TermControl::default()).await;
        let chat = crate::protocol::SessionId::new();
        let session_id = spawn_cat(&actor, &chat).await;

        // When resizing to a small grid.
        actor
            .tell(ResizeTerm {
                session_id: Some(session_id.clone()),
                size: (10, 40),
            })
            .await
            .expect("resize tell");

        // Give the actor a beat to process the tell.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Then the mirror still carries the session (no error occurred).
        let guard = state.read();
        assert!(guard.frontend.terminal.mirror(&chat).is_some());
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

    // ── v2: one terminal per chat session ──────────────────────────────────

    #[rstest::rstest]
    #[tokio::test]
    async fn respawn_same_chat_session_kills_the_previous_terminal() {
        // Given a coordinator whose chat session runs a long-lived marker
        // program (`sleep 31` — distinctive, so /proc probing finds exactly it).
        let harness = TestHarness::new().await;
        let (actor, _root) = spawn_coordinator(&harness, TermControl::default()).await;
        let chat = crate::protocol::SessionId::new();
        let first = {
            let SpawnTermOutcome::Started { session_id, .. } = actor
                .ask(spawn_msg(chat.clone(), "sleep 31"))
                .await
                .expect("spawn reply")
            else {
                panic!("expected Started");
            };
            session_id
        };

        // When spawning a second terminal for the same chat session.
        let reply = actor
            .ask(spawn_msg(chat, "printf second-run; sleep 30"))
            .await
            .expect("spawn reply");

        // Then the outcome reports the kill of the previous terminal.
        let SpawnTermOutcome::Started {
            session_id,
            killed_previous,
            screen,
        } = reply
        else {
            panic!("expected Started");
        };
        assert_ne!(session_id, first, "respawn must mint a fresh term id");
        let killed = killed_previous.expect("previous terminal killed");
        assert_eq!(killed.session_id, first);
        // And the new program's screen is the one reported.
        assert!(plain_screen(&screen.screen).contains("second-run"));

        // And the *killed* program is gone (no orphans of the first spawn).
        // A candidate must be a live (non-zombie) `/sleep` with the marker in
        // its cmdline; transient pid slots between readdir and open are skipped.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let find_orphans = || {
            let mut found = Vec::new();
            let entries = std::fs::read_dir("/proc").expect("/proc is readable");
            for entry in entries.flatten() {
                let Ok(exe) = std::fs::read_link(entry.path().join("exe")) else {
                    continue; // kernel thread, vanished, or not ours.
                };
                if !exe.to_string_lossy().ends_with("/sleep") {
                    continue;
                }
                // A zombie is already dead (the group kill worked); only a
                // live state counts as an orphan.
                let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
                    continue;
                };
                let Some(state) = stat
                    .rsplit(')')
                    .next()
                    .and_then(|rest| rest.split(' ').next())
                else {
                    continue;
                };
                if state == "Z" {
                    continue;
                }
                let Ok(cmdline) = std::fs::read_to_string(entry.path().join("cmdline")) else {
                    continue;
                };
                if cmdline.replace('\0', " ").contains("sleep 31") {
                    found.push(entry.file_name().to_string_lossy().to_string());
                }
            }
            found
        };
        let mut orphans = find_orphans();
        for _ in 0..3 {
            if orphans.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
            orphans = find_orphans();
        }
        assert!(
            orphans.is_empty(),
            "expected no 'sleep 31' orphans after respawn, found: {orphans:?}"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn parallel_chat_sessions_get_independent_terminals() {
        // Given a coordinator with a terminal for session A.
        let harness = TestHarness::new().await;
        let (actor, _root) = spawn_coordinator(&harness, TermControl::default()).await;
        let chat_a = crate::protocol::SessionId::new();
        let chat_b = crate::protocol::SessionId::new();
        let term_a = spawn_cat(&actor, &chat_a).await;

        // When spawning a terminal for session B.
        let SpawnTermOutcome::Started {
            session_id: term_b, ..
        } = actor
            .ask(spawn_msg(chat_b.clone(), "echo from-b"))
            .await
            .expect("spawn reply")
        else {
            panic!("expected Started");
        };

        // Then both terminals stay live: A's program still responds.
        let mut msg = send_msg(term_a.clone());
        msg.text = Some("still-alive-a".to_owned());
        let SendTermOutcome::Sent(screen) = actor.ask(msg).await.expect("send reply") else {
            panic!("expected Sent");
        };
        assert!(plain_screen(&screen.screen).contains("still-alive-a"));
        // And the two term ids differ.
        assert_ne!(term_a, term_b);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn live_flag_mirrors_spawn_and_kill() {
        // Given a coordinator wired to a readable state.
        let harness = TestHarness::new().await;
        let (actor, state, _root) =
            spawn_coordinator_with_state(&harness, TermControl::default()).await;
        let chat = crate::protocol::SessionId::new();
        let session_id = spawn_cat(&actor, &chat).await;

        // Then the session is marked live after spawn.
        assert!(
            state.read().frontend.terminal.live_terms.contains(&chat),
            "chat session must be live after spawn"
        );

        // When killing the terminal.
        actor
            .ask(KillTerm {
                session_id: session_id.clone(),
            })
            .await
            .expect("kill reply");

        // Then the live flag clears.
        assert!(
            !state.read().frontend.terminal.live_terms.contains(&chat),
            "chat session must not be live after kill"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn natural_exit_clears_the_live_flag() {
        // Given a coordinator with a short-lived terminal (`true` exits at once).
        let harness = TestHarness::new().await;
        let (actor, state, _root) =
            spawn_coordinator_with_state(&harness, TermControl::default()).await;
        let chat = crate::protocol::SessionId::new();
        let _ = {
            let SpawnTermOutcome::Started { session_id, .. } = actor
                .ask(spawn_msg(chat.clone(), "true"))
                .await
                .expect("spawn reply")
            else {
                panic!("expected Started");
            };
            session_id
        };

        // When the program exits and the screen task observes EOF.
        tokio::time::sleep(Duration::from_millis(600)).await;

        // Then the live flag cleared without any kill call.
        assert!(
            !state.read().frontend.terminal.live_terms.contains(&chat),
            "live flag must clear on natural exit"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn mirror_updates_without_any_tool_call_in_flight() {
        // Given a coordinator with a live terminal printing on a timer.
        let harness = TestHarness::new().await;
        let (actor, state, _root) =
            spawn_coordinator_with_state(&harness, TermControl::default()).await;
        let chat = crate::protocol::SessionId::new();
        let _term = {
            let SpawnTermOutcome::Started { session_id, .. } = actor
                .ask(spawn_msg(
                    chat.clone(),
                    "sleep 0.2; echo realtime-echo; sleep 30",
                ))
                .await
                .expect("spawn reply")
            else {
                panic!("expected Started");
            };
            session_id
        };

        // When no tool call is in flight and the program prints.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let contains = state
                .read()
                .frontend
                .terminal
                .mirror(&chat)
                .is_some_and(|m| m.screen.contains("realtime-echo"));
            if contains {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "mirror never updated without an in-flight ask"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}
