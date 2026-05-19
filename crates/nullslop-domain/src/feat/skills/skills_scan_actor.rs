//! Skills scan configuration for the generic scan actor.
//!
//! Subscribes to [`ScanSkills`](crate::protocol::Command::ScanSkills) commands,
//! scans the injected skills directory on a blocking thread, writes results to
//! shared [`State`](crate::common::state::State), and emits
//! [`SkillsLoaded`](crate::protocol::Event::SkillsLoaded) events.

use serde::{Deserialize, Serialize};

use crate::common::actor::ActorContext;
use crate::common::actor::scan_actor::{ScanActor, ScanConfig};
use crate::common::app_paths::AppPaths;
use crate::common::state::State;
use crate::feat::skills::scan::scan_skills;
use crate::feat::skills::skill::Skill;
use crate::protocol::{Command, CommandMsg, Event, EventMsg};

/// Skills scan configuration for [`ScanActor`].
///
/// On `ScanSkills`, scans the injected directory for `*/SKILL.md` files,
/// writes results to `AppState.context.skills`, and emits `SkillsLoaded`.
pub struct SkillsScanConfig {
    /// Shared application state.
    state: State,
}

impl ScanConfig for SkillsScanConfig {
    type Output = Vec<Skill>;

    #[expect(
        clippy::expect_used,
        reason = "State must be injected via ctx.set_data before activate"
    )]
    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<ScanSkills>();
        let state = ctx
            .take_data::<State>()
            .expect("State must be injected via ctx.set_data()");
        Self { state }
    }

    fn is_rescan_command(command: &Command) -> bool {
        matches!(command, Command::ScanSkills)
    }

    fn scan(paths: &AppPaths) -> Vec<Skill> {
        scan_skills(&paths.skills_dir())
    }

    fn on_success(skills: Vec<Skill>, config: &Self, ctx: &ActorContext) {
        tracing::info!(count = skills.len(), "scanned agent skills");

        // Write skills to shared state.
        {
            let mut guard = config.state.write();
            guard.context.skills.clone_from(&skills);
        }

        let _ = ctx.send_event(Event::SkillsLoaded(SkillsLoaded {
            skills,
            error: None,
        }));
    }

    fn on_panic(join_error: tokio::task::JoinError, _config: &Self, ctx: &ActorContext) {
        tracing::error!("skills scan task panicked: {join_error}");
        let _ = ctx.send_event(Event::SkillsLoaded(SkillsLoaded {
            skills: vec![],
            error: Some(format!("skills scan task failed: {join_error}")),
        }));
    }
}

/// Type alias for the skills scan actor.
pub type SkillsScanActor = ScanActor<SkillsScanConfig>;

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
/// The actor knows its scan path from injected context data,
/// so this command has no payload.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("skills")]
pub struct ScanSkills;

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
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

        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("skills-scan-test", sink.clone() as Arc<dyn MessageSink>);
        let state = State::new(AppState::default());
        ctx.set_data(AppPaths::new_in(dir.path()));
        ctx.set_data(state.clone());
        let mut actor = SkillsScanActor::activate(&mut ctx);

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

        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("skills-scan-test", sink.clone() as Arc<dyn MessageSink>);
        let state = State::new(AppState::default());
        ctx.set_data(AppPaths::new_in(dir.path()));
        ctx.set_data(state);
        let mut actor = SkillsScanActor::activate(&mut ctx);

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

        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("skills-scan-test", sink.clone() as Arc<dyn MessageSink>);
        let state = State::new(AppState::default());
        ctx.set_data(AppPaths::new_in(dir.path()));
        ctx.set_data(state);
        let mut actor = SkillsScanActor::activate(&mut ctx);

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

        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("skills-scan-test", sink.clone() as Arc<dyn MessageSink>);
        let state = State::new(AppState::default());
        ctx.set_data(AppPaths::new_in(dir.path()));
        ctx.set_data(state);
        let mut actor = SkillsScanActor::activate(&mut ctx);

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
