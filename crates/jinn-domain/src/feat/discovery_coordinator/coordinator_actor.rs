//! Discovery coordinator actor.
//!
//! Coalesces the three per-resource scan events —
//! [`SkillsLoaded`](crate::feat::skills::skills_scan_actor::SkillsLoaded),
//! [`PromptTemplatesLoaded`](crate::feat::provider::protocol::event::PromptTemplatesLoaded),
//! [`ContextFilesLoaded`](crate::feat::context::protocol::event::ContextFilesLoaded) — per
//! session into a single [`SessionDiscoverySettled`] event.
//!
//! Each session has a pending latch holding the three resource contributions plus a
//! start instant. When all three arrive, the coordinator emits `SessionDiscoverySettled`
//! immediately (`delayed: None`). If the 3000ms safety-net timer elapses first, it
//! emits anyway with `delayed: Some("discovery delayed by <missing>")` — important for
//! slow disk setups (e.g. ZFS raidz2 on spinners) where one scan can stall.
//!
//! All latch mutation happens inside the actor's single-threaded mailbox. The
//! event-arrival path self-sends a [`DiscoveryDirectMsg::Record`] so the `HashMap` is
//! never touched from the event-dispatch context.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::SessionId;
use crate::common::actor::{Actor, ActorContext, ActorEnvelope, ActorRef};
use crate::common::state::State;
use crate::feat::context::protocol::event::ContextFilesLoaded;
use crate::feat::discovery_coordinator::session_discovery_settled::{
    DiscoverySnapshot, SessionDiscoverySettled,
};
use crate::feat::provider::protocol::event::PromptTemplatesLoaded;
use crate::feat::skills::skills_scan_actor::SkillsLoaded;
use crate::protocol::Event;

/// Safety-net window. If not all three resource events arrive within this duration,
/// the coordinator settles anyway with a `delayed` reason.
const SAFETY_NET: Duration = Duration::from_millis(3000);

/// Direct messages the coordinator sends to itself.
///
/// `Record` carries one resource's contribution from the event-dispatch context into
/// the single-threaded mailbox. `CheckTimeout` is sent by the spawned safety-net
/// task.
pub enum DiscoveryDirectMsg {
    /// Records that one resource's scan settled for a session.
    Record {
        /// The session whose resource settled.
        session_id: SessionId,
        /// The resource contribution to merge into the session's pending slot.
        snapshot: ResourceSnapshot,
    },
    /// Safety-net timer fired for a session; settle it if still pending.
    CheckTimeout {
        /// The session the timer was armed for.
        session_id: SessionId,
        /// The `started_at` captured when the timer was armed; used to detect a
        /// superseded slot (a new trigger that reset `started_at`).
        started_at: Instant,
    },
}

/// One resource's contribution to a session's pending slot.
#[derive(Clone)]
pub enum ResourceSnapshot {
    /// Skills discovery settled.
    Skills {
        /// Number of discovered skills.
        count: usize,
        /// Error message if the scan failed.
        error: Option<String>,
    },
    /// Prompt discovery settled.
    Prompts {
        /// Number of discovered prompts.
        count: usize,
        /// Error message if the scan failed.
        error: Option<String>,
    },
    /// Context-files discovery settled.
    Context {
        /// Number of discovered context files (AGENTS.md / CLAUDE.md).
        count: usize,
        /// Error message if the scan failed.
        error: Option<String>,
    },
}

impl ResourceSnapshot {
    /// Extracts the (count, error) pair for snapshot assembly.
    fn counts(&self) -> (usize, &Option<String>) {
        match self {
            ResourceSnapshot::Skills { count, error }
            | ResourceSnapshot::Prompts { count, error }
            | ResourceSnapshot::Context { count, error } => (*count, error),
        }
    }
}

/// Per-session pending latch tracking which resources have settled.
#[derive(Default)]
struct PendingSlot {
    skills: Option<ResourceSnapshot>,
    prompts: Option<ResourceSnapshot>,
    context: Option<ResourceSnapshot>,
    started_at: Option<Instant>,
}

impl PendingSlot {
    /// Returns `true` once all three resources have reported.
    fn all_present(&self) -> bool {
        self.skills.is_some() && self.prompts.is_some() && self.context.is_some()
    }

    /// Builds the settled snapshot from the arrived contributions.
    fn to_snapshot(&self) -> DiscoverySnapshot {
        DiscoverySnapshot {
            skill_count: self.skills.as_ref().map_or(0, |s| s.counts().0),
            prompt_count: self.prompts.as_ref().map_or(0, |s| s.counts().0),
            context_file_count: self.context.as_ref().map_or(0, |s| s.counts().0),

            skill_error: self.skills.as_ref().and_then(|s| s.counts().1.clone()),
            prompt_error: self.prompts.as_ref().and_then(|s| s.counts().1.clone()),
            context_error: self.context.as_ref().and_then(|s| s.counts().1.clone()),
        }
    }

    /// Lists the resources that have not yet reported, for the delayed reason.
    fn missing_names(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.skills.is_none() {
            missing.push("skills");
        }
        if self.prompts.is_none() {
            missing.push("prompts");
        }
        if self.context.is_none() {
            missing.push("context");
        }
        missing
    }
}

/// Dependencies for [`DiscoveryCoordinatorActor`].
pub struct DiscoveryCoordinatorActorDeps {
    /// Shared application state (currently unused but kept for parity with other
    /// scan actors and future enrichment from state).
    #[allow(dead_code, reason = "kept for future use")]
    pub state: State,
}

/// Coalesces the three `*Loaded` events per session into one settled event.
///
/// See the [module docs](self) for the coalescing algorithm and safety-net timer.
pub struct DiscoveryCoordinatorActor {
    /// Pending per-session latches.
    pending: HashMap<SessionId, PendingSlot>,
    /// Self-ref for arming the safety-net timer and routing `Record` messages.
    self_ref: ActorRef<DiscoveryDirectMsg>,
}

impl Actor for DiscoveryCoordinatorActor {
    type Message = DiscoveryDirectMsg;
    type Deps = DiscoveryCoordinatorActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description("Coalesces discovery scans into SessionDiscoverySettled");

        ctx.subscribe_event::<SkillsLoaded>();
        ctx.subscribe_event::<PromptTemplatesLoaded>();
        ctx.subscribe_event::<ContextFilesLoaded>();

        #[expect(
            clippy::expect_used,
            reason = "self-ref is injected by spawn before activate"
        )]
        let self_ref = ctx
            .take_actor_ref::<DiscoveryDirectMsg>()
            .expect("DiscoveryCoordinatorActor requires self-ref injection");

        let _ = deps.state;
        Self {
            pending: HashMap::new(),
            self_ref,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => {
                self.on_loaded_event(&event);
            }
            ActorEnvelope::Direct(DiscoveryDirectMsg::Record {
                session_id,
                snapshot,
            }) => {
                self.on_record(session_id, snapshot, ctx);
            }
            ActorEnvelope::Direct(DiscoveryDirectMsg::CheckTimeout {
                session_id,
                started_at,
            }) => {
                self.on_check_timeout(session_id, started_at, ctx);
            }
            ActorEnvelope::Command(_) | ActorEnvelope::System(_) => {}
        }
    }

    async fn shutdown(self) {}
}

impl DiscoveryCoordinatorActor {
    /// Extracts a resource contribution from a `*Loaded` event and routes it into
    /// the mailbox as a `Record` so latch mutation is single-threaded.
    fn on_loaded_event(&self, event: &Event) {
        let Some((session_id, snapshot)) = Self::snapshot_for_event(event) else {
            return;
        };
        let _ = self.self_ref.send(DiscoveryDirectMsg::Record {
            session_id,
            snapshot,
        });
    }

    /// Maps a `*Loaded` event to `(session_id, ResourceSnapshot)`, if recognized.
    fn snapshot_for_event(event: &Event) -> Option<(SessionId, ResourceSnapshot)> {
        match event {
            Event::SkillsLoaded(SkillsLoaded {
                session_id,
                skills,
                error,
            }) => Some((
                session_id.clone(),
                ResourceSnapshot::Skills {
                    count: skills.len(),
                    error: error.clone(),
                },
            )),
            Event::PromptTemplatesLoaded(PromptTemplatesLoaded {
                session_id,
                templates,
                error,
            }) => Some((
                session_id.clone(),
                ResourceSnapshot::Prompts {
                    count: templates.len(),
                    error: error.clone(),
                },
            )),
            Event::ContextFilesLoaded(ContextFilesLoaded {
                session_id,
                files,
                error,
            }) => Some((
                session_id.clone(),
                ResourceSnapshot::Context {
                    count: files.len(),
                    error: error.clone(),
                },
            )),
            _ => None,
        }
    }

    /// Records one resource's arrival; emits settled when all three are present,
    /// or arms the safety-net timer on first arrival.
    fn on_record(&mut self, session_id: SessionId, snapshot: ResourceSnapshot, ctx: &ActorContext) {
        let slot = self.pending.entry(session_id.clone()).or_default();
        match &snapshot {
            ResourceSnapshot::Skills { .. } => slot.skills = Some(snapshot),
            ResourceSnapshot::Prompts { .. } => slot.prompts = Some(snapshot),
            ResourceSnapshot::Context { .. } => slot.context = Some(snapshot),
        }

        if slot.all_present() {
            // All three arrived: settle immediately, no delay.
            let snapshot = slot.to_snapshot();
            self.pending.remove(&session_id);
            Self::emit_settled(session_id, snapshot, None, ctx);
            return;
        }

        if slot.started_at.is_none() {
            // First arrival for this trigger: arm the safety-net timer.
            let started_at = Instant::now();
            slot.started_at = Some(started_at);
            self.arm_timer(session_id, started_at);
        }
    }

    /// Safety-net handler: settle if the slot is still pending and unchanged.
    fn on_check_timeout(&mut self, session_id: SessionId, started_at: Instant, ctx: &ActorContext) {
        let Some(slot) = self.pending.get(&session_id) else {
            return;
        };
        if slot.started_at != Some(started_at) || slot.all_present() {
            return;
        }
        let missing = slot.missing_names();
        let snapshot = slot.to_snapshot();
        let reason = format!("discovery delayed by {}", missing.join(", "));
        self.pending.remove(&session_id);
        Self::emit_settled(session_id, snapshot, Some(reason), ctx);
    }

    /// Emits the coalesced settled event for a session.
    fn emit_settled(
        session_id: SessionId,
        snapshot: DiscoverySnapshot,
        delayed: Option<String>,
        ctx: &ActorContext,
    ) {
        let _ = ctx.send_event(Event::SessionDiscoverySettled(SessionDiscoverySettled {
            session_id,
            snapshot,
            delayed,
        }));
    }

    /// Spawns the 3000ms safety-net task for a session.
    fn arm_timer(&self, session_id: SessionId, started_at: Instant) {
        let self_ref = self.self_ref.clone();
        tokio::spawn(async move {
            tokio::time::sleep(SAFETY_NET).await;
            let _ = self_ref.send(DiscoveryDirectMsg::CheckTimeout {
                session_id,
                started_at,
            });
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use std::path::PathBuf;
    use std::sync::Arc;

    use crate::SessionId;
    use crate::common::actor::{Actor, ActorContext, ActorEnvelope, MessageSink, RecordingSink};
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::context::env_context::ContextFile;
    use crate::feat::context::protocol::event::ContextFilesLoaded;
    use crate::feat::context::protocol::prompt_template::PromptTemplate;
    use crate::feat::provider::protocol::event::PromptTemplatesLoaded;
    use crate::feat::skills::skills_scan_actor::SkillsLoaded;
    use crate::feat::skills::{Skill, SkillSource};
    use crate::protocol::Event;

    /// Constructs the coordinator via `activate`, giving it a self-ref wired to
    /// a recording mailbox. Returns the actor, the sink (bus events), and a
    /// channel to drive self-messages back into `handle`.
    fn build() -> (
        super::DiscoveryCoordinatorActor,
        Arc<RecordingSink>,
        kanal::Receiver<ActorEnvelope<super::DiscoveryDirectMsg>>,
        ActorContext,
    ) {
        let sink = Arc::new(RecordingSink::new());
        let (tx, rx) = kanal::unbounded::<ActorEnvelope<super::DiscoveryDirectMsg>>();
        let self_ref = crate::common::actor::ActorRef::new(tx);
        let mut ctx = ActorContext::new(
            "discovery-coordinator-test",
            sink.clone() as Arc<dyn MessageSink>,
        );
        ctx.set_actor_ref(self_ref);
        let actor = super::DiscoveryCoordinatorActor::activate(
            super::DiscoveryCoordinatorActorDeps {
                state: State::new(AppState::default()),
            },
            &mut ctx,
        );
        (actor, sink, rx, ctx)
    }

    /// Pumps self-messages produced by `on_loaded_event` back through `handle`
    /// until the mailbox drains, plus a few yields for async spawn tasks.
    async fn drain(
        actor: &mut super::DiscoveryCoordinatorActor,
        rx: &kanal::Receiver<ActorEnvelope<super::DiscoveryDirectMsg>>,
        ctx: &ActorContext,
    ) {
        loop {
            tokio::task::yield_now().await;
            match rx.try_recv() {
                Ok(Some(msg)) => actor.handle(msg, ctx).await,
                Ok(None) | Err(_) => break,
            }
        }
    }

    fn skill_named(name: &str) -> Skill {
        Skill {
            name: name.to_string(),
            description: String::new(),
            body: String::new(),
            file_path: PathBuf::new(),
            base_dir: PathBuf::new(),
            source: SkillSource::default(),
        }
    }

    fn prompt_named(name: &str) -> PromptTemplate {
        PromptTemplate {
            name: name.to_string(),
            description: String::new(),
            body: String::new(),
        }
    }

    fn context_at(path: &str) -> ContextFile {
        ContextFile {
            path: PathBuf::from(path),
            content: String::new(),
        }
    }

    fn skills_loaded(id: &SessionId, n: usize) -> Event {
        Event::SkillsLoaded(SkillsLoaded {
            session_id: id.clone(),
            skills: (0..n).map(|i| skill_named(&format!("skill-{i}"))).collect(),
            error: None,
        })
    }

    fn prompts_loaded(id: &SessionId, n: usize) -> Event {
        Event::PromptTemplatesLoaded(PromptTemplatesLoaded {
            session_id: id.clone(),
            templates: (0..n)
                .map(|i| prompt_named(&format!("prompt-{i}")))
                .collect(),
            error: None,
        })
    }

    fn context_loaded(id: &SessionId, n: usize) -> Event {
        Event::ContextFilesLoaded(ContextFilesLoaded {
            session_id: id.clone(),
            files: (0..n)
                .map(|i| context_at(&format!("/p{i}/AGENTS.md")))
                .collect(),
            error: None,
        })
    }

    fn settled_count(sink: &RecordingSink) -> usize {
        sink.events()
            .iter()
            .filter(|e| matches!(e, Event::SessionDiscoverySettled(_)))
            .count()
    }

    fn first_settled(
        sink: &RecordingSink,
    ) -> crate::feat::discovery_coordinator::SessionDiscoverySettled {
        sink.events()
            .into_iter()
            .find_map(|e| match e {
                Event::SessionDiscoverySettled(s) => Some(s),
                _ => None,
            })
            .expect("at least one settled event")
    }

    #[tokio::test]
    async fn one_resource_does_not_emit_settled() {
        // Given a coordinator.
        let (mut actor, sink, rx, ctx) = build();

        // When only skills loaded arrives.
        let id = SessionId::new();
        actor
            .handle(ActorEnvelope::Event(skills_loaded(&id, 2)), &ctx)
            .await;
        drain(&mut actor, &rx, &ctx).await;

        // Then no settled event yet.
        assert_eq!(settled_count(&sink), 0, "one resource must not settle");
    }

    #[tokio::test]
    async fn two_resources_do_not_emit_settled() {
        // Given a coordinator.
        let (mut actor, sink, rx, ctx) = build();

        // When skills + prompts arrive.
        let id = SessionId::new();
        actor
            .handle(ActorEnvelope::Event(skills_loaded(&id, 2)), &ctx)
            .await;
        drain(&mut actor, &rx, &ctx).await;
        actor
            .handle(ActorEnvelope::Event(prompts_loaded(&id, 1)), &ctx)
            .await;
        drain(&mut actor, &rx, &ctx).await;

        // Then no settled event yet.
        assert_eq!(settled_count(&sink), 0, "two resources must not settle");
    }

    #[tokio::test]
    async fn all_three_emit_one_settled_with_no_delay() {
        // Given a coordinator.
        let (mut actor, sink, rx, ctx) = build();

        // When all three arrive in any order.
        let id = SessionId::new();
        actor
            .handle(ActorEnvelope::Event(context_loaded(&id, 1)), &ctx)
            .await;
        drain(&mut actor, &rx, &ctx).await;
        actor
            .handle(ActorEnvelope::Event(skills_loaded(&id, 3)), &ctx)
            .await;
        drain(&mut actor, &rx, &ctx).await;
        actor
            .handle(ActorEnvelope::Event(prompts_loaded(&id, 2)), &ctx)
            .await;
        drain(&mut actor, &rx, &ctx).await;

        // Then exactly one settled event, with no delay.
        assert_eq!(settled_count(&sink), 1);
        let settled = first_settled(&sink);
        assert!(settled.delayed.is_none());
        assert_eq!(settled.snapshot.skill_count, 3);
        assert_eq!(settled.snapshot.prompt_count, 2);
        assert_eq!(settled.snapshot.context_file_count, 1);
    }

    #[tokio::test]
    async fn failed_scan_still_counts_as_settled() {
        // Given a coordinator.
        let (mut actor, sink, rx, ctx) = build();

        // When the skills scan reports an error but the others succeed.
        let id = SessionId::new();
        actor
            .handle(
                ActorEnvelope::Event(Event::SkillsLoaded(SkillsLoaded {
                    session_id: id.clone(),
                    skills: vec![],
                    error: Some("disk error".into()),
                })),
                &ctx,
            )
            .await;
        drain(&mut actor, &rx, &ctx).await;
        actor
            .handle(ActorEnvelope::Event(prompts_loaded(&id, 0)), &ctx)
            .await;
        drain(&mut actor, &rx, &ctx).await;
        actor
            .handle(ActorEnvelope::Event(context_loaded(&id, 0)), &ctx)
            .await;
        drain(&mut actor, &rx, &ctx).await;

        // Then settled fires with the error flag set.
        let settled = first_settled(&sink);
        assert_eq!(settled.snapshot.skill_error.as_deref(), Some("disk error"));
    }

    #[tokio::test]
    async fn two_sessions_settle_independently() {
        // Given a coordinator.
        let (mut actor, sink, rx, ctx) = build();

        // When two sessions each receive all three events.
        for id in [SessionId::new(), SessionId::new()] {
            actor
                .handle(ActorEnvelope::Event(skills_loaded(&id, 1)), &ctx)
                .await;
            drain(&mut actor, &rx, &ctx).await;
            actor
                .handle(ActorEnvelope::Event(prompts_loaded(&id, 1)), &ctx)
                .await;
            drain(&mut actor, &rx, &ctx).await;
            actor
                .handle(ActorEnvelope::Event(context_loaded(&id, 1)), &ctx)
                .await;
            drain(&mut actor, &rx, &ctx).await;
        }

        // Then two settled events, one per session.
        assert_eq!(settled_count(&sink), 2);
    }

    #[tokio::test]
    async fn second_trigger_resets_and_emits_again() {
        // Given a coordinator with one settled trigger.
        let (mut actor, sink, rx, ctx) = build();
        let id = SessionId::new();
        for ev in [
            skills_loaded(&id, 1),
            prompts_loaded(&id, 1),
            context_loaded(&id, 1),
        ] {
            actor.handle(ActorEnvelope::Event(ev), &ctx).await;
            drain(&mut actor, &rx, &ctx).await;
        }
        assert_eq!(settled_count(&sink), 1);

        // When a second trigger fires all three again.
        for ev in [
            skills_loaded(&id, 4),
            prompts_loaded(&id, 5),
            context_loaded(&id, 6),
        ] {
            actor.handle(ActorEnvelope::Event(ev), &ctx).await;
            drain(&mut actor, &rx, &ctx).await;
        }

        // Then a second settled event reflects the new counts.
        let settled = {
            let events = sink.events();
            events
                .iter()
                .rev()
                .find_map(|e| match e {
                    Event::SessionDiscoverySettled(s) => Some(s),
                    _ => None,
                })
                .expect("settled")
                .clone()
        };
        assert_eq!(settled.snapshot.skill_count, 4);
        assert_eq!(settled.snapshot.prompt_count, 5);
        assert_eq!(settled.snapshot.context_file_count, 6);
    }

    #[tokio::test(start_paused = true)]
    async fn safety_net_timer_fires_delayed_settled() {
        // Given a coordinator.
        let (mut actor, sink, rx, ctx) = build();

        // When only skills + prompts arrive (context missing).
        let id = SessionId::new();
        actor
            .handle(ActorEnvelope::Event(skills_loaded(&id, 1)), &ctx)
            .await;
        drain(&mut actor, &rx, &ctx).await;
        actor
            .handle(ActorEnvelope::Event(prompts_loaded(&id, 1)), &ctx)
            .await;
        drain(&mut actor, &rx, &ctx).await;
        assert_eq!(settled_count(&sink), 0);

        // And the 3000ms safety-net timer elapses.
        tokio::time::advance(std::time::Duration::from_millis(3000)).await;
        drain(&mut actor, &rx, &ctx).await;

        // Then settled fires with a delayed reason naming the missing context.
        let settled = first_settled(&sink);
        let reason = settled.delayed.as_ref().expect("delayed reason set");
        assert!(
            reason.contains("context"),
            "reason should name the missing resource: {reason}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn all_three_just_under_timeout_settles_without_delay() {
        // Given a coordinator.
        let (mut actor, sink, rx, ctx) = build();

        // When all three arrive just under the 3000ms safety-net window.
        let id = SessionId::new();
        actor
            .handle(ActorEnvelope::Event(skills_loaded(&id, 1)), &ctx)
            .await;
        drain(&mut actor, &rx, &ctx).await;
        actor
            .handle(ActorEnvelope::Event(prompts_loaded(&id, 1)), &ctx)
            .await;
        drain(&mut actor, &rx, &ctx).await;
        actor
            .handle(ActorEnvelope::Event(context_loaded(&id, 1)), &ctx)
            .await;
        drain(&mut actor, &rx, &ctx).await;

        // Then settled fires exactly once with no delay.
        assert_eq!(settled_count(&sink), 1);
        assert!(
            first_settled(&sink).delayed.is_none(),
            "settled under the window must not be delayed"
        );

        // And advancing past 3000ms does not double-fire (stale-guard no-ops).
        tokio::time::advance(std::time::Duration::from_millis(3100)).await;
        drain(&mut actor, &rx, &ctx).await;
        assert_eq!(
            settled_count(&sink),
            1,
            "timer must not re-emit after settle"
        );
    }
}
