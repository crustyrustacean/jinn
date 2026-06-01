//! Skills scan actor - scans and loads agent skills on command.
//!
//! Subscribes to [`ScanSkills`](crate::protocol::Command::ScanSkills) commands,
//! scans the skills directory on a blocking thread, writes results to shared
//! [`State`](crate::common::state::State), and emits
//! [`SkillsLoaded`](crate::protocol::Event::SkillsLoaded) events.

use serde::{Deserialize, Serialize};

use crate::common::actor::scan_actor::NoDirectMsg;
use crate::common::actor::{Actor, ActorContext, ActorEnvelope};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::skills::scan::scan_skills;
use crate::feat::skills::skill::Skill;
use crate::protocol::{Command, CommandMsg, Event, EventMsg};

/// Dependencies for [`SkillsScanActor`].
pub struct SkillsScanActorDeps {
    /// Runtime services.
    pub services: Services,
    /// Shared application state.
    pub state: State,
}

/// Scans and loads agent skills on `ScanSkills`.
///
/// On command, scans the skills directory for `*/SKILL.md` files,
/// writes results to `AppState.context.skills`, and emits `SkillsLoaded`.
pub struct SkillsScanActor {
    /// Runtime services.
    services: Services,
    /// Shared application state.
    state: State,
}

impl Actor for SkillsScanActor {
    type Message = NoDirectMsg;
    type Deps = SkillsScanActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description("Scans and loads agent skills from ~/.agents/skills");
        ctx.subscribe_command::<ScanSkills>();
        Self {
            services: deps.services,
            state: deps.state,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<NoDirectMsg>, ctx: &ActorContext) {
        if let ActorEnvelope::Command(command) = msg {
            self.handle_command(&command, ctx).await;
        }
    }
}

impl SkillsScanActor {
    /// Dispatches incoming commands.
    async fn handle_command(&mut self, command: &Command, ctx: &ActorContext) {
        if matches!(command, Command::ScanSkills) {
            self.run_scan(ctx).await;
        }
    }

    /// Runs the blocking scan and emits the result.
    async fn run_scan(&self, ctx: &ActorContext) {
        let paths = self.services.paths.clone();
        let result = tokio::task::spawn_blocking(move || scan_skills(&paths.skills_dir())).await;

        match result {
            Ok(skills) => {
                tracing::info!(count = skills.len(), "scanned agent skills");

                // Write skills to shared state.
                {
                    let mut guard = self.state.write();
                    guard.context.skills.clone_from(&skills);
                }

                let _ = ctx.send_event(Event::SkillsLoaded(SkillsLoaded {
                    skills,
                    error: None,
                }));
            }
            Err(join_error) => {
                tracing::error!("skills scan task panicked: {join_error}");
                let _ = ctx.send_event(Event::SkillsLoaded(SkillsLoaded {
                    skills: vec![],
                    error: Some(format!("skills scan task failed: {join_error}")),
                }));
            }
        }
    }
}

/// Emitted when skills have been scanned and loaded.
///
/// On success, `skills` contains the discovered skills and `error` is `None`.
/// On failure, `skills` is empty and `error` contains a description.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("skills")]
pub struct SkillsLoaded {
    /// The discovered agent skills.
    pub skills: Vec<Skill>,
    /// Error message if scanning failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Command to trigger a skills scan.
///
/// The actor knows its scan path from deps, so this command has no payload.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("skills")]
pub struct ScanSkills;

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use std::sync::Arc;

    use crate::common::actor::{Actor, ActorContext, ActorEnvelope, MessageSink, RecordingSink};
    use crate::common::app_paths::AppPaths;
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::protocol::Command;

    use super::*;

    fn find_skills_loaded(events: &[Event]) -> Option<&SkillsLoaded> {
        for evt in events {
            if let Event::SkillsLoaded(payload) = evt {
                return Some(payload);
            }
        }
        None
    }

    fn create_actor(
        dir: &tempfile::TempDir,
        state: State,
    ) -> (SkillsScanActor, Arc<RecordingSink>, ActorContext) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("skills-scan-test", sink.clone() as Arc<dyn MessageSink>);
        let mut services = crate::common::services::test_services::TestServices::builder()
            .paths(AppPaths::new_in(dir.path()))
            .build();
        let deps = SkillsScanActorDeps {
            services,
            state,
        };

        let actor = SkillsScanActor::activate(deps, &mut ctx);
        (actor, sink, ctx)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_skills_command_writes_to_app_state() {
        // Given an actor with a temp directory containing a skill.
        let dir = tempfile::tempdir().expect("create temp dir");
        let skills_base = dir.path().join(".agents/skills/test-skill");
        std::fs::create_dir_all(&skills_base).expect("create skill dir");
        std::fs::write(
            skills_base.join("SKILL.md"),
            "---\nname: test-skill\ndescription: A test skill\n---\n\n# Content",
        )
        .expect("write SKILL.md");

        let state = State::new(AppState::default());
        let (mut actor, _sink, ctx) = create_actor(&dir, state.clone());

        // When processing ScanSkills command.
        actor
            .handle(ActorEnvelope::Command(Command::ScanSkills), &ctx)
            .await;

        // Then skills are written to AppState.
        let guard = state.read();
        assert_eq!(guard.context.skills.len(), 1);
        assert_eq!(guard.context.skills[0].name, "test-skill");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_skills_command_emits_skills_loaded() {
        // Given an actor with a temp directory.
        let dir = tempfile::tempdir().expect("create temp dir");
        let state = State::new(AppState::default());
        let (mut actor, sink, ctx) = create_actor(&dir, state);

        // When processing ScanSkills command.
        actor
            .handle(ActorEnvelope::Command(Command::ScanSkills), &ctx)
            .await;

        // Then SkillsLoaded event is emitted.
        let events = sink.events();
        let loaded = find_skills_loaded(&events);
        assert!(loaded.is_some());
        let loaded = loaded.expect("should have SkillsLoaded");
        assert!(loaded.error.is_none());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_skills_empty_dir_emits_empty_loaded() {
        // Given an actor with an empty temp directory.
        let dir = tempfile::tempdir().expect("create temp dir");
        let state = State::new(AppState::default());
        let (mut actor, sink, ctx) = create_actor(&dir, state);

        // When processing ScanSkills command.
        actor
            .handle(ActorEnvelope::Command(Command::ScanSkills), &ctx)
            .await;

        // Then SkillsLoaded has empty skills list.
        let events = sink.events();
        let loaded = find_skills_loaded(&events).expect("should have SkillsLoaded");
        assert!(loaded.skills.is_empty());
        assert!(loaded.error.is_none());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_skills_nonexistent_dir_emits_empty_loaded() {
        // Given an actor with a nonexistent directory.
        let dir = tempfile::tempdir().expect("create temp dir");
        let state = State::new(AppState::default());
        let (mut actor, sink, ctx) = create_actor(&dir, state);

        // When processing ScanSkills command.
        actor
            .handle(ActorEnvelope::Command(Command::ScanSkills), &ctx)
            .await;

        // Then SkillsLoaded has empty skills list.
        let events = sink.events();
        let loaded = find_skills_loaded(&events).expect("should have SkillsLoaded");
        assert!(loaded.skills.is_empty());
        assert!(loaded.error.is_none());
    }
}
