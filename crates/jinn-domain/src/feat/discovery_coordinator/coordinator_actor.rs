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

use kameo::prelude::{Actor, ActorRef, Context, Message};

use crate::SessionId;
use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::state::State;
use crate::feat::context::protocol::event::ContextFilesLoaded;
use crate::feat::discovery_coordinator::session_discovery_settled::{
    DiscoverySnapshot, SessionDiscoverySettled,
};
use crate::feat::provider::protocol::event::PromptTemplatesLoaded;
use crate::feat::skills::skills_scan_actor::SkillsLoaded;

/// Safety-net window. If not all three resource events arrive within this duration,
/// the coordinator settles anyway with a `delayed` reason.
const SAFETY_NET: Duration = Duration::from_secs(3);

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
#[derive(Clone)]
pub struct DiscoveryCoordinatorActorDeps {
    /// Common actor dependencies.
    pub deps: ActorDeps,
    /// Shared application state (currently unused but kept for parity with other
    /// scan actors and future enrichment from state).
    pub state: State,
}

/// Coalesces the three `*Loaded` events per session into one settled event.
///
/// See the [module docs](self) for the coalescing algorithm and safety-net timer.
pub struct DiscoveryCoordinatorActor {
    /// Pending per-session latches.
    pending: HashMap<SessionId, PendingSlot>,
    /// Self-ref for arming the safety-net timer and routing `Record` messages.
    actor_ref: ActorRef<Self>,
    /// Bus for message routing.
    bus: crate::common::services::bus_service::BusService,
}

impl Actor for DiscoveryCoordinatorActor {
    type Args = DiscoveryCoordinatorActorDeps;
    type Error = std::convert::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        // Register on bus for the three resource-loaded events.
        args.deps
            .subscribe(actor_ref.clone().recipient::<SkillsLoaded>())
            .await;
        args.deps
            .subscribe(actor_ref.clone().recipient::<PromptTemplatesLoaded>())
            .await;
        let stored_ref = actor_ref.clone();
        args.deps
            .subscribe(actor_ref.recipient::<ContextFilesLoaded>())
            .await;

        let _ = args.state;
        Ok(Self {
            pending: HashMap::new(),
            actor_ref: stored_ref,
            bus: args.deps.services.bus.clone(),
        })
    }
}

impl BusPublish for DiscoveryCoordinatorActor {
    fn bus(&self) -> &crate::common::services::bus_service::BusService {
        &self.bus
    }
}

impl Message<SkillsLoaded> for DiscoveryCoordinatorActor {
    type Reply = ();

    async fn handle(&mut self, msg: SkillsLoaded, _ctx: &mut Context<Self, Self::Reply>) {
        let snapshot = ResourceSnapshot::Skills {
            count: msg.skills.len(),
            error: msg.error.clone(),
        };
        let _ = self
            .actor_ref
            .tell(DiscoveryDirectMsg::Record {
                session_id: msg.session_id.clone(),
                snapshot,
            })
            .await;
    }
}

impl Message<PromptTemplatesLoaded> for DiscoveryCoordinatorActor {
    type Reply = ();

    async fn handle(&mut self, msg: PromptTemplatesLoaded, _ctx: &mut Context<Self, Self::Reply>) {
        let snapshot = ResourceSnapshot::Prompts {
            count: msg.templates.len(),
            error: msg.error.clone(),
        };
        let _ = self
            .actor_ref
            .tell(DiscoveryDirectMsg::Record {
                session_id: msg.session_id.clone(),
                snapshot,
            })
            .await;
    }
}

impl Message<ContextFilesLoaded> for DiscoveryCoordinatorActor {
    type Reply = ();

    async fn handle(&mut self, msg: ContextFilesLoaded, _ctx: &mut Context<Self, Self::Reply>) {
        let snapshot = ResourceSnapshot::Context {
            count: msg.files.len(),
            error: msg.error.clone(),
        };
        let _ = self
            .actor_ref
            .tell(DiscoveryDirectMsg::Record {
                session_id: msg.session_id.clone(),
                snapshot,
            })
            .await;
    }
}

impl Message<DiscoveryDirectMsg> for DiscoveryCoordinatorActor {
    type Reply = ();

    async fn handle(&mut self, msg: DiscoveryDirectMsg, _ctx: &mut Context<Self, Self::Reply>) {
        match msg {
            DiscoveryDirectMsg::Record {
                session_id,
                snapshot,
            } => self.on_record(session_id, snapshot).await,
            DiscoveryDirectMsg::CheckTimeout {
                session_id,
                started_at,
            } => self.on_check_timeout(session_id, started_at).await,
        }
    }
}

impl DiscoveryCoordinatorActor {
    /// Records one resource's arrival; emits settled when all three are present,
    /// or arms the safety-net timer on first arrival.
    async fn on_record(&mut self, session_id: SessionId, snapshot: ResourceSnapshot) {
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
            self.publish(SessionDiscoverySettled {
                session_id,
                snapshot,
                delayed: None,
            })
            .await;
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
    async fn on_check_timeout(&mut self, session_id: SessionId, started_at: Instant) {
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
        self.publish(SessionDiscoverySettled {
            session_id,
            snapshot,
            delayed: Some(reason),
        })
        .await;
    }

    /// Spawns the 3000ms safety-net task for a session.
    fn arm_timer(&self, session_id: SessionId, started_at: Instant) {
        let actor_ref = self.actor_ref.clone();
        tokio::spawn(async move {
            tokio::time::sleep(SAFETY_NET).await;
            let _ = actor_ref
                .tell(DiscoveryDirectMsg::CheckTimeout {
                    session_id,
                    started_at,
                })
                .await;
        });
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
    use std::path::PathBuf;

    use crate::SessionId;
    use crate::common::app_state::AppState;
    use crate::common::bus::test_harness::{Recorder, TestHarness, await_recorded};
    use crate::common::state::State;
    use crate::feat::context::env_context::ContextFile;
    use crate::feat::context::protocol::event::ContextFilesLoaded;
    use crate::feat::context::protocol::prompt_template::PromptTemplate;
    use crate::feat::discovery_coordinator::session_discovery_settled::SessionDiscoverySettled;
    use crate::feat::provider::protocol::event::PromptTemplatesLoaded;
    use crate::feat::skills::skills_scan_actor::SkillsLoaded;
    use crate::feat::skills::{Skill, SkillSource};

    use super::{DiscoveryCoordinatorActor, DiscoveryCoordinatorActorDeps};

    fn skill_named(name: &str) -> Skill {
        Skill {
            name: name.to_owned(),
            description: String::new(),
            body: String::new(),
            file_path: PathBuf::new(),
            base_dir: PathBuf::new(),
            source: SkillSource::default(),
        }
    }

    fn prompt_named(name: &str) -> PromptTemplate {
        PromptTemplate {
            name: name.to_owned(),
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

    async fn build_harness() -> (
        TestHarness,
        kameo::prelude::ActorRef<DiscoveryCoordinatorActor>,
        kameo::prelude::ActorRef<Recorder<SessionDiscoverySettled>>,
    ) {
        let harness = TestHarness::new().await;
        let actor = harness
            .spawn_actor::<DiscoveryCoordinatorActor>(DiscoveryCoordinatorActorDeps {
                deps: harness.actor_deps().await,
                state: State::new(AppState::default()),
            })
            .await;
        let recorder = harness.spawn_recorder::<SessionDiscoverySettled>().await;
        (harness, actor, recorder)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn one_resource_does_not_emit_settled() {
        // Given a coordinator.
        let (_harness, actor, recorder) = build_harness().await;

        // When only skills loaded arrives.
        let id = SessionId::new();
        actor
            .tell(SkillsLoaded {
                session_id: id,
                skills: vec![skill_named("s-0"), skill_named("s-1")],
                error: None,
            })
            .await
            .expect("tell");

        // Then no settled event yet.
        let messages = await_recorded(&recorder, 0, std::time::Duration::from_millis(100)).await;
        assert_eq!(messages.len(), 0, "one resource must not settle");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn two_resources_do_not_emit_settled() {
        // Given a coordinator.
        let (_harness, actor, recorder) = build_harness().await;

        // When skills + prompts arrive.
        let id = SessionId::new();
        actor
            .tell(SkillsLoaded {
                session_id: id.clone(),
                skills: vec![skill_named("s-0"), skill_named("s-1")],
                error: None,
            })
            .await
            .expect("tell");
        actor
            .tell(PromptTemplatesLoaded {
                session_id: id,
                templates: vec![prompt_named("p-0")],
                error: None,
            })
            .await
            .expect("tell");

        // Then no settled event yet.
        let messages = await_recorded(&recorder, 0, std::time::Duration::from_millis(100)).await;
        assert_eq!(messages.len(), 0, "two resources must not settle");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn all_three_emit_one_settled_with_no_delay() {
        // Given a coordinator.
        let (_harness, actor, recorder) = build_harness().await;

        // When all three arrive.
        let id = SessionId::new();
        actor
            .tell(ContextFilesLoaded {
                session_id: id.clone(),
                files: vec![context_at("/p0/AGENTS.md")],
                error: None,
            })
            .await
            .expect("tell");
        actor
            .tell(SkillsLoaded {
                session_id: id.clone(),
                skills: vec![skill_named("s-0"), skill_named("s-1"), skill_named("s-2")],
                error: None,
            })
            .await
            .expect("tell");
        actor
            .tell(PromptTemplatesLoaded {
                session_id: id,
                templates: vec![prompt_named("p-0"), prompt_named("p-1")],
                error: None,
            })
            .await
            .expect("tell");

        // Then exactly one settled event, with no delay.
        let messages = await_recorded(&recorder, 1, std::time::Duration::from_secs(1)).await;
        assert_eq!(messages.len(), 1);
        assert!(messages[0].delayed.is_none());
        assert_eq!(messages[0].snapshot.skill_count, 3);
        assert_eq!(messages[0].snapshot.prompt_count, 2);
        assert_eq!(messages[0].snapshot.context_file_count, 1);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn failed_scan_still_counts_as_settled() {
        // Given a coordinator.
        let (_harness, actor, recorder) = build_harness().await;

        // When the skills scan reports an error but the others succeed.
        let id = SessionId::new();
        actor
            .tell(SkillsLoaded {
                session_id: id.clone(),
                skills: vec![],
                error: Some("disk error".into()),
            })
            .await
            .expect("tell");
        actor
            .tell(PromptTemplatesLoaded {
                session_id: id.clone(),
                templates: vec![],
                error: None,
            })
            .await
            .expect("tell");
        actor
            .tell(ContextFilesLoaded {
                session_id: id,
                files: vec![],
                error: None,
            })
            .await
            .expect("tell");

        // Then settled fires with the error flag set.
        let messages = await_recorded(&recorder, 1, std::time::Duration::from_secs(1)).await;
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].snapshot.skill_error.as_deref(),
            Some("disk error")
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn two_sessions_settle_independently() {
        // Given a coordinator.
        let (_harness, actor, recorder) = build_harness().await;

        // When two sessions each receive all three events.
        for _ in [SessionId::new(), SessionId::new()] {
            let id = SessionId::new();
            actor
                .tell(SkillsLoaded {
                    session_id: id.clone(),
                    skills: vec![skill_named("s")],
                    error: None,
                })
                .await
                .expect("tell");
            actor
                .tell(PromptTemplatesLoaded {
                    session_id: id.clone(),
                    templates: vec![prompt_named("p")],
                    error: None,
                })
                .await
                .expect("tell");
            actor
                .tell(ContextFilesLoaded {
                    session_id: id,
                    files: vec![context_at("/p/AGENTS.md")],
                    error: None,
                })
                .await
                .expect("tell");
        }

        // Then two settled events, one per session.
        let messages = await_recorded(&recorder, 1, std::time::Duration::from_secs(1)).await;
        assert_eq!(messages.len(), 2);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn second_trigger_resets_and_emits_again() {
        // Given a coordinator.
        let (_harness, actor, recorder) = build_harness().await;

        let id = SessionId::new();
        // First trigger.
        actor
            .tell(SkillsLoaded {
                session_id: id.clone(),
                skills: vec![skill_named("s")],
                error: None,
            })
            .await
            .expect("tell");
        actor
            .tell(PromptTemplatesLoaded {
                session_id: id.clone(),
                templates: vec![prompt_named("p")],
                error: None,
            })
            .await
            .expect("tell");
        actor
            .tell(ContextFilesLoaded {
                session_id: id.clone(),
                files: vec![context_at("/p/AGENTS.md")],
                error: None,
            })
            .await
            .expect("tell");

        let first = await_recorded(&recorder, 1, std::time::Duration::from_secs(1)).await;
        assert_eq!(first.len(), 1);

        // Second trigger with different counts.
        actor
            .tell(SkillsLoaded {
                session_id: id.clone(),
                skills: (0..4).map(|i| skill_named(&format!("s-{i}"))).collect(),
                error: None,
            })
            .await
            .expect("tell");
        actor
            .tell(PromptTemplatesLoaded {
                session_id: id.clone(),
                templates: (0..5).map(|i| prompt_named(&format!("p-{i}"))).collect(),
                error: None,
            })
            .await
            .expect("tell");
        actor
            .tell(ContextFilesLoaded {
                session_id: id,
                files: (0..6)
                    .map(|i| context_at(&format!("/p{i}/AGENTS.md")))
                    .collect(),
                error: None,
            })
            .await
            .expect("tell");

        // Then a second settled event reflects the new counts.
        let second = await_recorded(&recorder, 1, std::time::Duration::from_secs(1)).await;
        assert_eq!(second.len(), 1);
        let last = &second[0];
        assert_eq!(last.snapshot.skill_count, 4);
        assert_eq!(last.snapshot.prompt_count, 5);
        assert_eq!(last.snapshot.context_file_count, 6);
    }

    #[rstest::rstest]
    #[tokio::test(start_paused = true)]
    async fn safety_net_timer_fires_delayed_settled() {
        // Given a coordinator.
        let (_harness, actor, recorder) = build_harness().await;

        // When only skills + prompts arrive (context missing).
        let id = SessionId::new();
        actor
            .tell(SkillsLoaded {
                session_id: id.clone(),
                skills: vec![skill_named("s")],
                error: None,
            })
            .await
            .expect("tell");
        actor
            .tell(PromptTemplatesLoaded {
                session_id: id,
                templates: vec![prompt_named("p")],
                error: None,
            })
            .await
            .expect("tell");

        // Wait a bit for record processing.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // And the 3000ms safety-net timer elapses.
        tokio::time::advance(std::time::Duration::from_secs(3)).await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Then settled fires with a delayed reason naming the missing context.
        let messages = await_recorded(&recorder, 1, std::time::Duration::from_secs(1)).await;
        assert_eq!(messages.len(), 1);
        let reason = messages[0].delayed.as_ref().expect("delayed reason set");
        assert!(
            reason.contains("context"),
            "reason should name the missing resource: {reason}"
        );
    }

    #[rstest::rstest]
    #[tokio::test(start_paused = true)]
    async fn all_three_just_under_timeout_settles_without_delay() {
        // Given a coordinator.
        let (_harness, actor, recorder) = build_harness().await;

        // When all three arrive just under the 3000ms safety-net window.
        let id = SessionId::new();
        actor
            .tell(SkillsLoaded {
                session_id: id.clone(),
                skills: vec![skill_named("s")],
                error: None,
            })
            .await
            .expect("tell");
        actor
            .tell(PromptTemplatesLoaded {
                session_id: id.clone(),
                templates: vec![prompt_named("p")],
                error: None,
            })
            .await
            .expect("tell");
        actor
            .tell(ContextFilesLoaded {
                session_id: id,
                files: vec![context_at("/p/AGENTS.md")],
                error: None,
            })
            .await
            .expect("tell");

        // Then settled fires exactly once with no delay.
        let messages = await_recorded(&recorder, 1, std::time::Duration::from_secs(1)).await;
        assert_eq!(messages.len(), 1);
        assert!(
            messages[0].delayed.is_none(),
            "settled under the window must not be delayed"
        );

        // And advancing past 3000ms does not double-fire (stale-guard no-ops).
        tokio::time::advance(std::time::Duration::from_millis(3100)).await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let recorded2 = await_recorded(&recorder, 0, std::time::Duration::from_millis(100)).await;
        assert_eq!(recorded2.len(), 0, "timer must not re-emit after settle");
    }
}
