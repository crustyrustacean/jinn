//! The turn ledger — the sole authority for turn-scoped admission.
//!
//! One mutex'd map of live turns keyed by session; no entry means `Idle`.
//! Every caller that performs a turn-scoped mutation asks the ledger with
//! [`TurnLedger::call`], which checks identity and runs the work under one
//! lock acquisition, so a caller can never act on a stale token: it never
//! sees the decision, only the outcome.
//!
//! The ledger replaces the phase machine's single stored phase value. A
//! single value encodes an event ordering the bus does not guarantee —
//! every published message rides its own spawned task — and that encoded
//! order was the root cause of the dropped-tool-result bug this module
//! retires. The ledger instead tracks **identity and work**: which turn is
//! live, which stream is in flight, which tool calls are still open, and
//! what happens when the work set empties. Arrival order stops mattering
//! because "the turn is over" is an order-independent check.
//!
//! # The finalized state table
//!
//! Admission is a pure function of the entry and the request:
//!
//! ```text
//! admit ⇔ token.turn == live.turn
//!       && token.stream == live.stream   (StreamScoped requests only)
//!       && state ∈ req.states
//! ```
//!
//! There is no `Idle` requirement anywhere — [`TurnLedger::mint`] *is* the
//! `Idle` → busy edge. The request kinds:
//!
//! | Request              | Admits when                                        |
//! |----------------------|----------------------------------------------------|
//! | `Req::StreamScoped`  | live turn, live stream, not cancelling             |
//! | `Req::TurnScoped`    | live turn, not cancelling                          |
//! | `Req::Any`           | live turn — cancelling included (drain, real out)  |
//!
//! A live turn entry carries three facts and one intent:
//!
//! | Field    | Meaning                                                     |
//! |----------|-------------------------------------------------------------|
//! | `turn`   | the token: per-session monotonic counter                    |
//! | `stream` | the provider stream in flight, if any                       |
//! | `open`   | tool calls registered at dispatch, closed by their result   |
//! | `next`   | what happens when the work set empties                      |
//!
//! The work-set rule (order independence): work is **registered by whoever
//! starts it** — all N calls of a batch join `open` in the same ledger
//! operation that dispatches them — and **retired by the consumer**: each
//! result closes its call. A turn is over iff `stream.is_none() &&
//! open.is_empty()`; whichever of the two happens last runs `next`:
//!
//! | `next`      | Settle behavior                                              |
//! |-------------|--------------------------------------------------------------|
//! | `Next::Continue` | entry kept, `next` reset to `End`; the tool loop's      |
//! |                  | continuation re-joins the same turn under a new stream  |
//! | `Next::End`      | entry removed; the caller reports the completed turn    |
//! | `Next::Cancel`   | entry removed; the caller sweeps still-pending tool     |
//! |                  | results to failure and reports the cancellation once    |
//!
//! The phase is **derived, never stored** — [`PhaseView`] reads the entry:
//!
//! | View         | Derived from                          |
//! |--------------|---------------------------------------|
//! | `Idle`       | no entry                              |
//! | `Streaming`  | entry with a stream in flight         |
//! | `Working`    | entry with open calls, stream retired |
//! | `Cancelling` | entry whose `next` is `Cancel`        |
//!
//! # Deadlock note
//!
//! [`TurnLedger::call`] holds the ledger lock while `f` runs. `f` must be
//! synchronous, short, and must not call back into the ledger.

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;

use jinn_core_types::SessionId;
use jinn_core_types::ToolCallId;
use jinn_core_types::turn_identity::StreamId;
use jinn_core_types::turn_identity::TurnToken;
use parking_lot::Mutex;

/// What happens when a turn's work set empties.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// The tool loop continues: the caller dispatches a continuation that
    /// re-joins the same turn under a fresh stream.
    Continue,
    /// The turn ends: the caller removes nothing (the ledger already did)
    /// and reports the completed turn.
    End,
    /// The turn was cancelled: the caller sweeps still-pending tool results
    /// and reports the cancellation once.
    Cancel,
}

/// Why a ledger operation refused.
///
/// Reasons, not booleans: every refusal is loggable, and "not yet"
/// ([`Refused::StaleTurn`], a race the turn may still win) is distinguishable
/// from "never" ([`Refused::NoTurn`], a straggler with nothing left to land on).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// No turn is live for the session — nothing was ever minted, or the
    /// live turn already settled.
    NoTurn,
    /// The event's token names a turn that is not the live one.
    StaleTurn,
    /// The event's stream is not the live stream (a retry swapped streams,
    /// or the stream already retired).
    StaleStream,
    /// The turn is draining from a cancel, and this request does not
    /// survive a cancel.
    Cancelled,
    /// A tool call completion named a call that was never registered, or
    /// was already closed.
    UnknownCall,
}

/// Which admission rule a [`TurnLedger::call`] applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Req {
    /// Dies with the stream: stream tokens and completions. Carries the
    /// stream the event rode in on; refused by a stream swap (retry) and by
    /// a cancel.
    StreamScoped(StreamId),
    /// Dies with the turn: work scoped to the turn but not to one stream.
    /// Refused by a cancel.
    TurnScoped,
    /// The live turn, cancelling included: cancel-drain work and real tool
    /// output, which wins over the synthesized cancellation.
    Any,
}

/// The mint kinds a dispatch can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MintKind {
    /// A fresh turn: always mints a new token (never during a cancel drain,
    /// which must not be clobbered).
    FreshTurn,
    /// A resume of the live turn (stall retry, rule-intercept resume):
    /// joins the live turn, mints a fresh stream.
    Resume,
    /// A tool-loop continuation: joins the live turn, mints a fresh stream.
    ToolContinuation,
}

/// The grant a successful [`TurnLedger::mint`] returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnGrant {
    /// The turn's identity, carried by every event of the turn.
    pub turn: TurnToken,
    /// The stream's identity, carried by every stream-scoped event of this
    /// dispatch.
    pub stream: StreamId,
}

/// The live turn's identity, as [`TurnLedger::live`] reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveTurn {
    /// The live turn's token.
    pub turn: TurnToken,
    /// The stream in flight, if any.
    pub stream: Option<StreamId>,
    /// The turn is draining a cancel (`Next::Cancel`): no new work may
    /// join it, and its resume paths must refuse.
    pub draining: bool,
}

/// The derived phase: what the UI and the guards read. The one phase
/// representation in the workspace, shared through `jinn-slices`.
///
/// Derived from the live entry at read time; nothing stores it, so it can
/// never disagree with the ledger.
pub use jinn_slices::PhaseView;

/// One live turn: identity, in-flight stream, open work, and the settle
/// intent.
#[derive(Debug)]
struct Turn {
    /// The token this turn answers to.
    token: TurnToken,
    /// The provider stream in flight, if any. Retired by its completion.
    stream: Option<StreamId>,
    /// Tool calls registered at dispatch and closed by their results.
    open: HashSet<ToolCallId>,
    /// Calls whose result already closed this turn. Registration is the
    /// start of work, so a call here is never re-registered: a late ack
    /// naming a closed call must not resurrect it and hold the turn open
    /// forever (a fast tool's result can beat the `ToolUse` ack through a
    /// deep mailbox — the bug-21 order).
    closed: HashSet<ToolCallId>,
    /// What runs when the work set empties.
    next: Next,
    /// Whether the work set emptied and the intent was consumed — the turn
    /// is over and only its removal remains.
    settled: bool,
}

impl Turn {
    /// Whether the turn's work is done: no stream in flight and no open
    /// calls. Order-independent by construction — whichever fact retires
    /// last observes this.
    fn work_set_empty(&self) -> bool {
        self.stream.is_none() && self.open.is_empty()
    }
}

/// Per-session ledger record: the entry plus the monotonic counters that
/// outlive it.
#[derive(Debug, Default)]
struct SessionTurns {
    /// Last minted turn number; zero until the first fresh turn.
    turn_seq: u64,
    /// Last minted stream number; zero until the first dispatch.
    stream_seq: u64,
    /// The live turn, if any.
    entry: Option<Turn>,
}

impl SessionTurns {
    /// Mints the next stream id.
    fn next_stream(&mut self) -> StreamId {
        self.stream_seq += 1;
        StreamId::from_counter(self.stream_seq)
    }
}

/// The outcome of a settle that emptied a turn's work set.
///
/// The caller owns the side effects (publishing, sweeping); the ledger owns
/// the state change — `Continue` keeps the entry, `End` and `Cancel` remove
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settlement {
    /// What the emptied turn asked for.
    pub next: Next,
}

/// The outcome of a cancel on a live turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// The drain started: the caller arms its synthesis and emission paths.
    Started,
    /// The drain started **and** the work set was already empty — the cancel
    /// was the drain's last fact. The caller sweeps and emits immediately.
    StartedAndSettled,
}

/// The turn ledger: shared serialized state behind a cheap clone.
///
/// Clones share one map; every operation takes the lock once.
#[derive(Debug, Clone, Default)]
pub struct TurnLedger {
    inner: Arc<Mutex<HashMap<SessionId, SessionTurns>>>,
}

impl TurnLedger {
    /// Runs `f` iff `token` names the live turn and the ledger's state
    /// admits `req`.
    ///
    /// One lock acquisition spans check and execution: the operation is
    /// linearized, and the caller cannot act on a stale token because it
    /// never sees the decision — only [`Refused`] or the outcome.
    ///
    /// `f` runs under the ledger lock: synchronous, short, and never calling
    /// back into the ledger.
    ///
    /// # Errors
    ///
    /// [`Refused::NoTurn`] when the session has no live turn,
    /// [`Refused::StaleTurn`] when `token` does not name it, and
    /// [`Refused::Cancelled`] when the turn is draining. `f` never runs on
    /// an error.
    pub fn call<T, F>(
        &self,
        session_id: &SessionId,
        token: &TurnToken,
        req: Req,
        f: F,
    ) -> Result<T, Refused>
    where
        F: FnOnce() -> T,
    {
        let turns = self.inner.lock();
        admit(turns.get(session_id), *token, req)?;
        Ok(f())
    }

    /// Registers tool calls as open work and runs the dispatch — one lock
    /// acquisition spanning admission, registration, and `f`.
    ///
    /// This is the batch dispatch's entry point: a completion can never be
    /// processed before its registration lands, because registration and
    /// dispatch share the same critical section the completions must enter.
    ///
    /// # Errors
    ///
    /// [`Refused::NoTurn`], [`Refused::StaleTurn`], or
    /// [`Refused::Cancelled`] — the call ids are not registered and `f`
    /// never runs.
    pub fn register_and_dispatch<T, F>(
        &self,
        session_id: &SessionId,
        token: &TurnToken,
        call_ids: Vec<ToolCallId>,
        f: F,
    ) -> Result<T, Refused>
    where
        F: FnOnce() -> T,
    {
        let mut turns = self.inner.lock();
        admit(turns.get(session_id), *token, Req::TurnScoped)?;
        if let Some(entry) = turns.get_mut(session_id).and_then(|r| r.entry.as_mut()) {
            for id in call_ids {
                if !entry.closed.contains(&id) {
                    entry.open.insert(id);
                }
            }
        }
        Ok(f())
    }

    /// Mints or joins a turn for a dispatch.
    ///
    /// `FreshTurn` always mints a new token — except during a cancel drain,
    /// where it is refused rather than allowed to clobber the draining
    /// entry. `Resume` and `ToolContinuation` join the live turn under a
    /// fresh stream id, and are refused when no live turn exists (a resume
    /// whose turn was cancelled publishes nothing, because there is nothing
    /// left to run).
    #[must_use]
    pub fn mint(&self, session_id: &SessionId, kind: MintKind) -> Option<TurnGrant> {
        let mut turns = self.inner.lock();
        let record = turns.entry(session_id.clone()).or_default();
        match kind {
            MintKind::FreshTurn => {
                if record
                    .entry
                    .as_ref()
                    .is_some_and(|entry| entry.next == Next::Cancel)
                {
                    tracing::debug!(
                        session_id = %session_id,
                        "fresh turn refused: the session is draining a cancel"
                    );
                    return None;
                }
                record.turn_seq += 1;
                let stream = record.next_stream();
                record.entry = Some(Turn {
                    token: TurnToken::from_counter(record.turn_seq),
                    stream: Some(stream),
                    open: HashSet::new(),
                    closed: HashSet::new(),
                    next: Next::End,
                    settled: false,
                });
                Some(TurnGrant {
                    turn: TurnToken::from_counter(record.turn_seq),
                    stream,
                })
            }
            MintKind::Resume | MintKind::ToolContinuation => {
                let live = record.entry.is_some();
                let can_join = record
                    .entry
                    .as_ref()
                    .is_some_and(|entry| entry.next != Next::Cancel);
                if !live {
                    tracing::info!(
                        session_id = %session_id,
                        kind = ?kind,
                        "join refused: no live turn behind it"
                    );
                    return None;
                }
                if !can_join {
                    tracing::debug!(
                        session_id = %session_id,
                        kind = ?kind,
                        "join refused: the turn is draining a cancel"
                    );
                    return None;
                }
                let stream = record.next_stream();
                if let Some(entry) = record.entry.as_mut() {
                    entry.stream = Some(stream);
                }
                let Some(entry) = record.entry.as_mut() else {
                    // Unreachable: `can_join` checked a live entry above.
                    return None;
                };
                entry.stream = Some(stream);
                Some(TurnGrant {
                    turn: entry.token,
                    stream,
                })
            }
        }
    }

    /// Begins the cancel drain for a session's live turn.
    ///
    /// Idempotent: the first call flips `next` to [`Next::Cancel`] and
    /// reports the drain's start; later calls are no-ops, so the drain's own
    /// bookkeeping (the single "Canceled" emission, the pending sweep) runs
    /// once. No live turn means nothing to drain: a no-op.
    #[must_use]
    pub fn begin_cancel(&self, session_id: &SessionId) -> Option<CancelOutcome> {
        let mut turns = self.inner.lock();
        let outcome = match turns.get_mut(session_id).and_then(|r| r.entry.as_mut()) {
            Some(entry) if entry.next != Next::Cancel => {
                entry.next = Next::Cancel;
                // A cancel may itself be the last fact: a turn whose stream
                // and calls are already gone has nothing left to wait for.
                match Self::settle_if_empty(entry) {
                    Some(_) => Some(CancelOutcome::StartedAndSettled),
                    None => Some(CancelOutcome::Started),
                }
            }
            _ => None,
        };
        Self::sweep_settled(&mut turns);
        outcome
    }

    /// Retires the turn's in-flight stream: its completion landed.
    ///
    /// Admits under [`Req::Any`] — a completion is turn end-work, and the
    /// completion that ends a cancelled turn's stream is exactly the drain
    /// waiting to run — but only the live stream is admitted at all: a
    /// completion from a stream a retry swapped out is
    /// [`Refused::StaleStream`]. The settle intent is applied only when the
    /// turn is not already draining — a cancel's [`Next::Cancel`] is never
    /// clobbered. Returns the settlement when this retirement emptied the
    /// work set, so the caller can publish exactly once.
    ///
    /// # Errors
    ///
    /// [`Refused::NoTurn`] when the turn already ended,
    /// [`Refused::StaleTurn`] when `token` names an earlier turn, and
    /// [`Refused::StaleStream`] when `stream` is not the turn's in-flight
    /// stream — the retirement of a swapped-out generation.
    pub fn ack_stream_ended(
        &self,
        session_id: &SessionId,
        token: &TurnToken,
        stream: &StreamId,
        next: Next,
    ) -> Result<Option<Settlement>, Refused> {
        self.ack_stream_with_calls(session_id, token, stream, next, Vec::new())
    }

    /// As [`Self::ack_stream_ended`], registering `call_ids` as open work
    /// in the same critical section the stream retires in.
    ///
    /// A tool-use completion names the calls the model asked for, and the
    /// batch dispatching them can trail the ack on the bus. Registering at
    /// the ack — not only at the batch — is what makes the empty-set settle
    /// wait for the results: whichever of (batch, completion, result) lands
    /// last finds the same non-empty set until every result closes.
    ///
    /// # Errors
    ///
    /// As [`Self::ack_stream_ended`]: [`Refused::NoTurn`],
    /// [`Refused::StaleTurn`], or [`Refused::StaleStream`]; the calls are
    /// left unregistered.
    pub fn ack_stream_with_calls(
        &self,
        session_id: &SessionId,
        token: &TurnToken,
        stream: &StreamId,
        next: Next,
        call_ids: Vec<ToolCallId>,
    ) -> Result<Option<Settlement>, Refused> {
        let mut turns = self.inner.lock();
        admit(turns.get(session_id), *token, Req::Any)?;
        let entry = turns
            .get_mut(session_id)
            .and_then(|r| r.entry.as_mut())
            .ok_or(Refused::NoTurn)?;
        // The live stream retires; a stream that is not the in-flight one is
        // refused — a swapped-out generation's completion must not retire
        // its replacement.
        if entry.stream != Some(*stream) {
            return Err(Refused::StaleStream);
        }
        entry.stream = None;
        if entry.next != Next::Cancel {
            entry.next = next;
        }
        for id in call_ids {
            if !entry.closed.contains(&id) {
                entry.open.insert(id);
            }
        }
        let settled = Self::settle_if_empty(entry);
        Self::sweep_settled(&mut turns);
        Ok(settled)
    }

    /// Closes one open tool call: its result landed.
    ///
    /// Idempotent — a call already closed (or never registered) is a no-op
    /// returning `None`, so a real completion and a synthesized cancellation
    /// for the same call close it exactly once between them. Returns the
    /// settlement when this close emptied the work set.
    pub fn close_call(&self, session_id: &SessionId, call_id: &ToolCallId) -> Option<Settlement> {
        let mut turns = self.inner.lock();
        let settled = {
            let entry = turns.get_mut(session_id).and_then(|r| r.entry.as_mut())?;
            // The closure is remembered even when nothing was open: a result
            // that beats its registration (a fast tool racing the `ToolUse`
            // ack through a deep mailbox) must bar the registration, or the
            // re-opened call would hold the turn open forever.
            entry.closed.insert(call_id.clone());
            if !entry.open.remove(call_id) {
                return None;
            }
            Self::settle_if_empty(entry)
        };
        Self::sweep_settled(&mut turns);
        settled
    }

    /// Retires the live stream without a token, as a turn's own finished
    /// edge does (`StreamEndedFinished`): the stream is over, nothing more
    /// will arrive on it, and the turn ends.
    ///
    /// Returns the removed turn when the retirement emptied the work set
    /// (`None` when no live turn exists — already settled or never minted).
    /// Test seam for paths that drive the ledger directly; production paths
    /// go through `ack_stream_ended` with their own token.
    pub fn finish_streaming(&self, session_id: &SessionId) -> Option<Settlement> {
        let mut turns = self.inner.lock();
        let removed = {
            let entry = turns.get_mut(session_id).and_then(|r| r.entry.as_mut())?;
            entry.stream = None;
            if entry.next != Next::Cancel {
                entry.next = Next::End;
            }
            Self::settle_if_empty(entry)
        };
        Self::sweep_settled(&mut turns);
        removed
    }

    /// Removes a session's ledger record outright — the session was closed
    /// or deleted mid-turn.
    ///
    /// Later events for the session are refused as [`Refused::NoTurn`] rather
    /// than resolved against a ghost turn.
    pub fn remove(&self, session_id: &SessionId) {
        self.inner.lock().remove(session_id);
    }

    /// Derives the phase view for a session.
    ///
    /// A settled entry — its work set emptied and its intent consumed by the
    /// caller that observed the settlement — reads as [`PhaseView::Idle`]:
    /// the turn is over, whatever the removal timing.
    #[must_use]
    pub fn view(&self, session_id: &SessionId) -> PhaseView {
        let turns = self.inner.lock();
        match turns.get(session_id).and_then(|r| r.entry.as_ref()) {
            None => PhaseView::Idle,
            Some(entry) if entry.settled => PhaseView::Idle,
            Some(entry) => match entry.next {
                Next::Cancel => PhaseView::Cancelling,
                Next::Continue | Next::End => {
                    if entry.stream.is_some() {
                        PhaseView::Streaming
                    } else {
                        PhaseView::Working
                    }
                }
            },
        }
    }

    /// The live turn's open-call ids, snapshot under the lock.
    ///
    /// Cancel synthesis reads this to know which calls need a synthesized
    /// completion; the ids are exactly the batch the orchestrator has out.
    /// `None` when no live turn exists; empty when the calls all closed.
    #[must_use]
    pub fn open_calls(&self, session_id: &SessionId) -> Option<Vec<ToolCallId>> {
        let turns = self.inner.lock();
        Some(
            turns
                .get(session_id)
                .and_then(|r| r.entry.as_ref())?
                .open
                .iter()
                .cloned()
                .collect(),
        )
    }

    /// The live turn's identity: the token and stream a caller needs to
    /// build events that belong to the turn in flight.
    ///
    /// `None` when no live turn exists. `stream` is `None` when the turn's
    /// stream already retired (tools running).
    #[must_use]
    pub fn live(&self, session_id: &SessionId) -> Option<LiveTurn> {
        let turns = self.inner.lock();
        let entry = turns.get(session_id).and_then(|r| r.entry.as_ref())?;
        if entry.settled {
            return None;
        }
        Some(LiveTurn {
            turn: entry.token,
            stream: entry.stream,
            draining: entry.next == Next::Cancel,
        })
    }

    /// Marks a settled entry and sweeps the map, so the map stays bounded by
    /// live turns without a read path that writes.
    fn sweep_settled(turns: &mut HashMap<SessionId, SessionTurns>) {
        turns.retain(|_, record| record.entry.as_ref().is_none_or(|entry| !entry.settled));
    }

    /// Runs the settle intent when the work set has emptied.
    ///
    /// `Continue` keeps the entry (the continuation re-joins it) and resets
    /// the intent for the next round; `End` and `Cancel` leave the entry in
    /// place — removal happens in [`Self::settle`], which the caller that
    /// observed the settlement invokes after its side effects, so a publish
    /// and the removal never interleave with another operation's lock.
    fn settle_if_empty(entry: &mut Turn) -> Option<Settlement> {
        if !entry.work_set_empty() {
            return None;
        }
        let next = entry.next;
        match next {
            Next::Continue => {
                // The continuation re-joins the same entry; reset the intent
                // for the next round.
                entry.next = Next::End;
            }
            Next::End | Next::Cancel => {
                // The turn is over: mark it settled. The sweep removes the
                // entry at the settle paths' leisure; the view already reads
                // a settled entry as Idle.
                entry.settled = true;
                entry.next = Next::End;
            }
        }
        Some(Settlement { next })
    }
}

/// The admission check shared by every locking operation: `Ok(())` when the
/// request may proceed, one [`Refused`] reason otherwise.
fn admit(record: Option<&SessionTurns>, token: TurnToken, req: Req) -> Result<(), Refused> {
    let entry = record
        .and_then(|r| r.entry.as_ref())
        .ok_or(Refused::NoTurn)?;
    if entry.token != token {
        return Err(Refused::StaleTurn);
    }
    match req {
        Req::Any => Ok(()),
        Req::TurnScoped => {
            if entry.next == Next::Cancel {
                Err(Refused::Cancelled)
            } else {
                Ok(())
            }
        }
        Req::StreamScoped(event_stream) => {
            if entry.next == Next::Cancel {
                return Err(Refused::Cancelled);
            }
            if entry.stream == Some(event_stream) {
                Ok(())
            } else {
                Err(Refused::StaleStream)
            }
        }
    }
}

/// Mints or joins a turn through the ledger and writes the session's
/// `Streaming` projection.
///
/// The ledger is the admission authority; the phase is a display projection
/// written beside it so a renderer reads one value. A refused mint applies
/// nothing: the caller publishes nothing downstream.
///
/// Lives beside the ledger rather than in a slice because every dispatch
/// path (turn-dispatch's queue actor, the session actor's tool-loop
/// continuation) fuses the same pair, and no slice sits below both.
#[must_use]
pub async fn mint_dispatch(
    services: &crate::common::services::Services,
    state: &crate::common::state::State,
    session_id: &SessionId,
    kind: MintKind,
) -> Option<TurnGrant> {
    let grant = services.turn_ledger.mint(session_id, kind)?;
    let (old, new) = state.with_session(|view| {
        let session = view.session.map().get_or_create(session_id);
        let old = session.phase();
        session.begin_streaming();
        (old, session.phase())
    });
    if old != new {
        crate::common::phase_events::publish_phase_change(&services.bus, session_id, old, new)
            .await;
    }
    Some(grant)
}

/// Mints or joins a turn through the ledger with a stamp: as [`mint_dispatch`],
/// plus the in-flight-stream guard the stall watchdog reads.
#[must_use]
pub async fn mint_dispatch_with_stamp(
    services: &crate::common::services::Services,
    state: &crate::common::state::State,
    session_id: &SessionId,
    kind: MintKind,
    dispatched_at: jiff::Timestamp,
) -> Option<TurnGrant> {
    let grant = services.turn_ledger.mint(session_id, kind)?;
    let (old, new) = state.with_session(|view| {
        let session = view.session.map().get_or_create(session_id);
        let old = session.phase();
        session.begin_streaming();
        session.arm_stream(dispatched_at);
        (old, session.phase())
    });
    if old != new {
        crate::common::phase_events::publish_phase_change(&services.bus, session_id, old, new)
            .await;
    }
    Some(grant)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::MintKind;
    use super::Next;
    use super::PhaseView;
    use super::Refused;
    use super::Req;
    use super::Settlement;
    use super::TurnLedger;
    use jinn_core_types::SessionId;
    use jinn_core_types::ToolCallId;

    /// A session id for ledger tests, derived from a stable tag.
    fn session(tag: &str) -> SessionId {
        SessionId::new_v5_tag(tag)
    }

    /// A tool call id for ledger tests.
    fn call_id(raw: &str) -> ToolCallId {
        ToolCallId::from(raw)
    }

    #[rstest::rstest]
    #[test]
    fn mint_creates_turn_with_stream_and_end_intent() {
        // Given an idle ledger.
        let ledger = TurnLedger::default();
        let sid = session("tm1");

        // When a fresh turn mints.
        let grant = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");

        // Then the turn entry exists with a stream in flight, and the work
        // set empties toward ending the turn.
        assert_eq!(ledger.view(&sid), PhaseView::Streaming);
        assert_eq!(grant.turn.get(), 1);
        assert_eq!(grant.stream.get(), 1);
        // And a second fresh mint advances the counter: identity, not state.
        let second = ledger.mint(&sid, MintKind::FreshTurn).expect("second mint");
        assert_eq!(second.turn.get(), 2);
    }

    #[rstest::rstest]
    #[test]
    fn turn_scoped_call_refused_when_no_turn_is_live() {
        // Given an idle ledger.
        let ledger = TurnLedger::default();
        let sid = session("tm3");
        let token = super::TurnToken::from_counter(99);

        // When a turn-scoped call names the dead token.
        let result = ledger.call(&sid, &token, Req::TurnScoped, || 7);

        // Then the call is refused with NoTurn.
        assert_eq!(result.unwrap_err(), Refused::NoTurn);
    }

    #[rstest::rstest]
    #[test]
    fn stream_scoped_call_refused_for_a_swapped_stream() {
        // Given a live turn whose stream was replaced by a resume mint.
        let ledger = TurnLedger::default();
        let sid = session("tm4");
        let first = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");
        let resumed = ledger.mint(&sid, MintKind::Resume).expect("resume mint");
        assert_ne!(first.stream, resumed.stream, "a resume mints a new stream");

        // When a stream-scoped event arrives under the pre-retry stream.
        let result = ledger.call(&sid, &first.turn, Req::StreamScoped(first.stream), || ());

        // Then it is refused as a stale stream.
        assert_eq!(result.unwrap_err(), Refused::StaleStream);
    }

    #[rstest::rstest]
    #[test]
    fn turn_scoped_call_runs_the_closure_when_admitted() {
        // Given a live turn.
        let ledger = TurnLedger::default();
        let sid = session("tm-admit");
        let grant = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");

        // When a turn-scoped call names the live token.
        let ran = ledger.call(&sid, &grant.turn, Req::TurnScoped, || 41 + 1);

        // Then the closure ran and its outcome came back.
        assert_eq!(ran.expect("admitted"), 42);
    }

    #[rstest::rstest]
    #[test]
    fn register_and_dispatch_admits_the_batch_under_one_lock() {
        // Given a live turn.
        let ledger = TurnLedger::default();
        let sid = session("tm-register");
        let grant = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");

        // When a batch of two calls registers and dispatches.
        let dispatched = ledger.register_and_dispatch(
            &sid,
            &grant.turn,
            vec![call_id("tc-1"), call_id("tc-2")],
            || "dispatched",
        );

        // Then the dispatch ran.
        assert_eq!(dispatched.expect("admitted"), "dispatched");
    }

    #[rstest::rstest]
    #[test]
    fn completion_for_a_never_registered_call_is_a_no_op_close() {
        // Given a live turn with no open calls.
        let ledger = TurnLedger::default();
        let sid = session("tm7");
        let _grant = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");

        // When a completion closes a call that was never registered.
        let settled = ledger.close_call(&sid, &call_id("tc-ghost"));

        // Then nothing settles: an unregistered close cannot end a turn.
        assert!(settled.is_none());
    }

    #[rstest::rstest]
    #[test]
    fn duplicate_close_closes_exactly_once() {
        // Given a live turn holding one open call whose stream has retired
        // toward ending.
        let ledger = TurnLedger::default();
        let sid = session("tm8");
        let grant = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");
        let _ = ledger.register_and_dispatch(&sid, &grant.turn, vec![call_id("tc-1")], || ());
        ledger
            .ack_stream_ended(&sid, &grant.turn, &grant.stream, Next::End)
            .expect("live stream");

        // When the call closes twice.
        let first = ledger.close_call(&sid, &call_id("tc-1"));
        let second = ledger.close_call(&sid, &call_id("tc-1"));

        // Then only the first close settles the turn: the work set emptied
        // exactly once, and the duplicate is a no-op.
        assert_eq!(first.map(|s| s.next), Some(Next::End));
        assert!(second.is_none());
    }

    #[rstest::rstest]
    #[test]
    fn stream_retire_with_no_open_work_settles_the_intent() {
        // Given a live turn with no open calls and a Finished intent.
        let ledger = TurnLedger::default();
        let sid = session("tm-end");
        let grant = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");

        // When the stream retires.
        let settled = ledger
            .ack_stream_ended(&sid, &grant.turn, &grant.stream, Next::End)
            .expect("live stream");

        // Then the turn settles to End and the entry is gone.
        assert_eq!(settled.map(|s| s.next), Some(Next::End));
        assert_eq!(ledger.view(&sid), PhaseView::Idle);
    }

    #[rstest::rstest]
    #[test]
    fn tool_use_retire_keeps_the_entry_for_the_continuation() {
        // Given a live turn whose batch has already registered.
        let ledger = TurnLedger::default();
        let sid = session("tm-continue");
        let grant = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");
        let _ = ledger.register_and_dispatch(&sid, &grant.turn, vec![call_id("tc-1")], || ());

        // When the stream retires toward a continuation.
        let settled = ledger
            .ack_stream_ended(&sid, &grant.turn, &grant.stream, Next::Continue)
            .expect("live stream");

        // Then nothing settles yet — the calls are still open — and the
        // session reads Working.
        assert!(settled.is_none(), "open calls must hold the turn open");
        assert_eq!(ledger.view(&sid), PhaseView::Working);

        // And when the last call closes, the turn settles to Continue with
        // the entry kept for the continuation to re-join.
        let settled = ledger
            .close_call(&sid, &call_id("tc-1"))
            .expect("last call settles");
        assert_eq!(settled.next, Next::Continue);
        assert_ne!(ledger.view(&sid), PhaseView::Idle);
    }

    #[rstest::rstest]
    #[test]
    fn continuation_join_mints_a_fresh_stream_under_the_same_turn() {
        // Given a turn settled toward Continue.
        let ledger = TurnLedger::default();
        let sid = session("tm18");
        let grant = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");
        ledger
            .ack_stream_ended(&sid, &grant.turn, &grant.stream, Next::Continue)
            .expect("live stream");

        // When the continuation joins.
        let joined = ledger.mint(&sid, MintKind::ToolContinuation).expect("join");

        // Then it carries the same turn token under a new stream id.
        assert_eq!(joined.turn, grant.turn);
        assert_ne!(joined.stream, grant.stream);
    }

    #[rstest::rstest]
    #[test]
    fn join_refused_when_no_turn_is_live() {
        // Given an idle ledger.
        let ledger = TurnLedger::default();
        let sid = session("tm-join-refused");

        // When a continuation tries to join.
        let joined = ledger.mint(&sid, MintKind::ToolContinuation);

        // Then the join is refused: nothing is live to join.
        assert!(joined.is_none());
    }

    #[rstest::rstest]
    #[test]
    fn begin_cancel_flips_next_and_reports_once() {
        // Given a live turn.
        let ledger = TurnLedger::default();
        let sid = session("tm9-cancel");
        let _grant = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");

        // When the cancel begins — twice.
        let first = ledger.begin_cancel(&sid);
        let second = ledger.begin_cancel(&sid);

        // Then only the first cancel took effect, starting the drain.
        assert_eq!(first, Some(super::CancelOutcome::Started));
        assert_eq!(second, None, "the second cancel is a no-op");
        assert_eq!(ledger.view(&sid), PhaseView::Cancelling);
    }

    #[rstest::rstest]
    #[test]
    fn cancel_on_an_idle_session_is_a_no_op() {
        // Given an idle ledger.
        let ledger = TurnLedger::default();
        let sid = session("tm11-idle-cancel");

        // When the cancel begins.
        let began = ledger.begin_cancel(&sid);

        // Then nothing happened: there is no turn to drain.
        assert_eq!(began, None);
        assert_eq!(ledger.view(&sid), PhaseView::Idle);
    }

    #[rstest::rstest]
    #[test]
    fn cancel_may_itself_be_the_last_fact() {
        // Given a turn settled toward Continue — the continuation window,
        // where the entry stays live with an empty work set.
        let ledger = TurnLedger::default();
        let sid = session("tm-cancel-last");
        let grant = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");
        let _ = ledger.register_and_dispatch(&sid, &grant.turn, vec![call_id("tc-1")], || ());
        ledger
            .ack_stream_ended(&sid, &grant.turn, &grant.stream, Next::Continue)
            .expect("live stream");
        ledger
            .close_call(&sid, &call_id("tc-1"))
            .expect("the batch settles toward its continuation");

        // When the cancel lands in that window.
        let began = ledger.begin_cancel(&sid);

        // Then the cancel is the drain's last fact: it started the drain and
        // settled the turn in the same step.
        assert_eq!(began, Some(super::CancelOutcome::StartedAndSettled));
        assert_eq!(ledger.view(&sid), PhaseView::Idle);
    }

    #[rstest::rstest]
    #[test]
    fn cancel_never_clobbered_by_a_late_settle_intent() {
        // Given a live turn already draining.
        let ledger = TurnLedger::default();
        let sid = session("tm-cancel-intent");
        let grant = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");
        assert_eq!(
            ledger.begin_cancel(&sid),
            Some(super::CancelOutcome::Started)
        );

        // When the stream retires carrying a non-cancel intent.
        let settled = ledger
            .ack_stream_ended(&sid, &grant.turn, &grant.stream, Next::End)
            .expect("live stream during drain");

        // Then the settle still runs — but under the cancel's intent.
        assert_eq!(settled.map(|s| s.next), Some(Next::Cancel));
    }

    #[rstest::rstest]
    #[test]
    fn fresh_turn_refused_during_a_cancel_drain() {
        // Given a live turn draining from a cancel.
        let ledger = TurnLedger::default();
        let sid = session("tm12-drain");
        let _grant = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");
        assert_eq!(
            ledger.begin_cancel(&sid),
            Some(super::CancelOutcome::Started)
        );

        // When a fresh dispatch mints during the drain.
        let minted = ledger.mint(&sid, MintKind::FreshTurn);

        // Then the mint is refused: the drain must not be clobbered.
        assert!(minted.is_none());
        assert_eq!(ledger.view(&sid), PhaseView::Cancelling);
    }

    #[rstest::rstest]
    #[test]
    fn turn_scoped_work_refused_during_a_drain_but_drain_work_admitted() {
        // Given a live turn draining from a cancel.
        let ledger = TurnLedger::default();
        let sid = session("tm-any-during-cancel");
        let grant = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");
        assert_eq!(
            ledger.begin_cancel(&sid),
            Some(super::CancelOutcome::Started)
        );

        // When cancel-agnostic work asks under `Any`, it is admitted.
        let drained = ledger.call(&sid, &grant.turn, Req::Any, || "drained");
        assert_eq!(drained.expect("drain work survives cancel"), "drained");

        // And when plain turn-scoped work asks, it is refused.
        let plain = ledger.call(&sid, &grant.turn, Req::TurnScoped, || ());
        assert_eq!(plain.unwrap_err(), Refused::Cancelled);
    }

    #[rstest::rstest]
    #[test]
    fn remove_leaves_later_events_nowhere_to_land() {
        // Given a live turn for a session that is about to be deleted.
        let ledger = TurnLedger::default();
        let sid = session("tm15-remove");
        let grant = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");

        // When the session's record is removed.
        ledger.remove(&sid);

        // Then the view is idle and the old token is refused.
        assert_eq!(ledger.view(&sid), PhaseView::Idle);
        let result = ledger.call(&sid, &grant.turn, Req::Any, || ());
        assert_eq!(result.unwrap_err(), Refused::NoTurn);
    }

    #[rstest::rstest]
    #[test]
    fn stale_turn_token_refused_against_a_newer_turn() {
        // Given a session on its second turn.
        let ledger = TurnLedger::default();
        let sid = session("tm-stale-turn");
        let first = ledger.mint(&sid, MintKind::FreshTurn).expect("first mint");
        let second = ledger.mint(&sid, MintKind::FreshTurn).expect("second mint");

        // When the first turn's token asks.
        let result = ledger.call(&sid, &first.turn, Req::Any, || ());

        // Then it is refused as a stale turn, not a missing one.
        assert_eq!(result.unwrap_err(), Refused::StaleTurn);
        assert_ne!(first.turn, second.turn);
    }

    #[rstest::rstest]
    #[test]
    fn phase_view_derives_working_when_only_calls_remain() {
        // Given a turn whose stream retired while calls are open.
        let ledger = TurnLedger::default();
        let sid = session("tm19-working");
        let grant = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");
        let _ = ledger.register_and_dispatch(&sid, &grant.turn, vec![call_id("tc-1")], || ());
        ledger
            .ack_stream_ended(&sid, &grant.turn, &grant.stream, Next::Continue)
            .expect("live stream");

        // When the view is read.
        let view = ledger.view(&sid);

        // Then it derives Working — calls executing, stream gone.
        assert_eq!(view, PhaseView::Working);
    }

    #[rstest::rstest]
    #[test]
    fn settlement_only_fires_once_per_turn_round() {
        // Given a turn headed for Continue whose calls already closed.
        let ledger = TurnLedger::default();
        let sid = session("tm-settle-once");
        let grant = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");
        ledger
            .ack_stream_ended(&sid, &grant.turn, &grant.stream, Next::Continue)
            .expect("live stream");

        // When the stream retires a second time under the same (now
        // stale-but-equal) stream id.
        let again = ledger.ack_stream_ended(&sid, &grant.turn, &grant.stream, Next::Continue);

        // Then the retirement is refused — the stream is already gone — so
        // the settle cannot fire twice.
        assert_eq!(again.unwrap_err(), Refused::StaleStream);
    }

    #[rstest::rstest]
    #[test]
    fn settlement_type_carries_the_next_for_the_caller() {
        // Given a settled continue.
        let settlement = Settlement {
            next: Next::Continue,
        };

        // When the caller reads it.
        // Then it names the continuation.
        assert_eq!(settlement.next, Next::Continue);
    }

    #[rstest::rstest]
    #[test]
    fn resume_join_keeps_a_registered_call_open_for_its_late_completion() {
        // Given a live turn holding one open call — a tool dispatched before
        // the stream stalled (TM-20's shape).
        let ledger = TurnLedger::default();
        let sid = session("tm20-retry");
        let grant = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");
        let _ = ledger.register_and_dispatch(&sid, &grant.turn, vec![call_id("tc-1")], || ());

        // When a stall retry joins the turn under a new stream.
        let resumed = ledger.mint(&sid, MintKind::Resume).expect("resume mint");
        assert_ne!(
            resumed.stream, grant.stream,
            "the retry dispatches under a fresh stream id"
        );
        assert_eq!(resumed.turn, grant.turn, "the retry joins the same turn");

        // Then the call is still open — the retry must not orphan it.
        let live = ledger.live(&sid).expect("live turn after resume");
        assert_eq!(
            ledger.open_calls(&sid),
            Some(vec![call_id("tc-1")]),
            "an open call survives the retry join"
        );

        // And when the completion finally lands, it closes the call and
        // settles the turn under the retried stream's retirement.
        let settled_close = ledger.close_call(&sid, &call_id("tc-1"));
        assert!(
            settled_close.is_none(),
            "the retried stream is still in flight; a close alone cannot settle"
        );
        let settled = ledger
            .ack_stream_ended(&sid, &resumed.turn, &resumed.stream, Next::End)
            .expect("the retried stream retires");
        assert_eq!(settled.map(|s| s.next), Some(Next::End));
        assert_eq!(ledger.view(&sid), PhaseView::Idle);
        let _ = live;
    }

    #[rstest::rstest]
    #[test]
    fn every_batch_completion_and_tool_use_interleaving_converges_to_the_same_history() {
        // TM-23: the permutation property. The three facts of a tool round —
        // the batch dispatch, the stream's tool-use completion, and the tool
        // result — race on the bus, so their processing order must not decide
        // the turn's end state. "The turn is over" is an order-independent
        // check (stream retired AND work set empty).
        //
        // One order is causally unreachable — the result processing before
        // the batch that started the tool — but the ledger does not rely on
        // that: a closed call is never re-registered, so even an impossible
        // order could not wedge the turn. All six orders must converge to
        // the same state: settled toward Continue exactly once, the work set
        // emptied, the entry kept for the continuation.
        for order in [
            [0, 1, 2], // batch, tool-use, completion
            [0, 2, 1], // batch, completion, tool-use — bug #21's order
            [1, 0, 2], // tool-use, batch, completion
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            // Given a fresh ledger per ordering: identity carries across the
            // ordering, not shared mutable state.
            let ledger = TurnLedger::default();
            let sid = session("tm23-permutation");
            let grant = ledger
                .mint(&sid, MintKind::FreshTurn)
                .expect("fresh mint for the ordering");

            // When the round's facts land in this order. The tool-use step
            // is production's `ack_stream_with_calls`: the completion names
            // the calls, and registering them there is what keeps a batch
            // trailing the ack from settling an empty set.
            for step in order {
                match step {
                    0 => {
                        ledger
                            .register_and_dispatch(&sid, &grant.turn, vec![call_id("tc-1")], || ())
                            .expect("batch admits in every order");
                    }
                    1 => {
                        ledger
                            .ack_stream_with_calls(
                                &sid,
                                &grant.turn,
                                &grant.stream,
                                Next::Continue,
                                vec![call_id("tc-1")],
                            )
                            .expect("tool-use ack admits in every order");
                    }
                    _ => {
                        ledger.close_call(&sid, &call_id("tc-1"));
                    }
                }
            }

            // Then every order converges: the work set emptied exactly once
            // and settled toward the continuation, and the entry is kept for
            // it.
            let live = ledger.live(&sid).expect("entry kept for the continuation");
            assert_eq!(live.turn, grant.turn, "the continuation re-joins this turn");
            assert!(live.stream.is_none(), "the stream retired");
            assert_eq!(
                ledger.open_calls(&sid),
                Some(vec![]),
                "the work set emptied"
            );
            assert_eq!(
                ledger.view(&sid),
                PhaseView::Working,
                "the same view for every ordering"
            );
        }
    }

    #[rstest::rstest]
    #[test]
    fn result_landing_between_batch_and_tool_use_finalizes_in_place() {
        // TM-23's bug-21 instance, on the ledger alone: the result closes its
        // call while the stream is still in flight, and the later tool-use
        // ack settles — the finalize never blocks the turn.
        let ledger = TurnLedger::default();
        let sid = session("tm23-bug21");
        let grant = ledger.mint(&sid, MintKind::FreshTurn).expect("fresh mint");

        // When the batch dispatches, the result lands, and only then the
        // stream retires with its tool-use.
        let _ = ledger.register_and_dispatch(&sid, &grant.turn, vec![call_id("tc-1")], || ());
        assert!(
            ledger.close_call(&sid, &call_id("tc-1")).is_none(),
            "an in-flight stream holds the turn open past an early result"
        );
        let settled = ledger
            .ack_stream_ended(&sid, &grant.turn, &grant.stream, Next::Continue)
            .expect("the tool-use ack settles after the early result");

        // Then the settle names the continuation and the entry is kept for it.
        assert_eq!(
            settled.expect("an early result then the ack settles").next,
            Next::Continue
        );
        assert!(
            ledger.live(&sid).is_some(),
            "entry kept for the continuation"
        );
    }
}
