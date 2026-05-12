//! Skills scan actor — discovers and loads agent skills from disk.
//!
//! Subscribes to [`ScanSkills`](crate::protocol::Command::ScanSkills) commands,
//! scans the injected skills directory on a blocking thread, writes results to
//! shared [`State`](crate::common::state::State), and emits
//! [`SkillsLoaded`](crate::protocol::Event::SkillsLoaded) events.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::common::actor::{
    Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink, SystemMessage,
};
use crate::common::actor_host::{ActorSpawnResult, spawn_actor};
use crate::common::state::State;
use crate::feat::skills::scan::scan_skills;
use crate::feat::skills::skill::Skill;
use crate::protocol::{Command, Event, CommandMsg, EventMsg};

/// Direct message type for the skills scan actor (unused).
pub enum SkillsScanDirectMsg {}

/// Skills scan actor.
///
/// On `ScanSkills`, scans the injected directory for `*/SKILL.md` files,
/// writes results to `AppState.context.skills`, and emits `SkillsLoaded`.
pub struct SkillsScanActor {
    /// Directory to scan for skills.
    scan_path: PathBuf,
    /// Shared application state.
    state: State,
}

impl Actor for SkillsScanActor {
    type Message = SkillsScanDirectMsg;

    #[expect(
        clippy::expect_used,
        reason = "scan_path and State must be injected via ctx.set_data before activate"
    )]
    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<ScanSkills>();
        let scan_path = ctx
            .take_data::<PathBuf>()
            .expect("PathBuf must be injected via ctx.set_data()");
        let state = ctx
            .take_data::<State>()
            .expect("State must be injected via ctx.set_data()");
        Self { scan_path, state }
    }

    async fn handle(&mut self, msg: ActorEnvelope<SkillsScanDirectMsg>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(command) => self.handle_command(&command, ctx).await,
            ActorEnvelope::System(SystemMessage::ApplicationReady) => {
                ctx.announce_started();
            }
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::Event(_) | ActorEnvelope::Direct(_) | ActorEnvelope::Shutdown => {}
        }
    }

    async fn shutdown(self) {}
}

impl SkillsScanActor {
    /// Dispatches incoming commands.
    async fn handle_command(&mut self, command: &Command, ctx: &ActorContext) {
        match command {
            Command::ScanSkills => {
                self.scan(ctx).await;
            }
            _ => {}
        }
    }

    /// Scans the skills directory on a blocking thread and emits the result.
    async fn scan(&self, ctx: &ActorContext) {
        let scan_path = self.scan_path.clone();
        let result = tokio::task::spawn_blocking(move || scan_skills(&scan_path)).await;

        match result {
            Ok(skills) => {
                tracing::info!(count = skills.len(), "scanned agent skills");

                // Write skills to shared state.
                {
                    let mut guard = self.state.write();
                    guard.context.skills = skills.clone();
                }

                let _ = ctx.send_event(Event::SkillsLoaded {
                    payload: SkillsLoaded {
                        skills,
                        error: None,
                    },
                });
            }
            Err(join_err) => {
                tracing::error!("skills scan task panicked: {join_err}");
                let _ = ctx.send_event(Event::SkillsLoaded {
                    payload: SkillsLoaded {
                        skills: vec![],
                        error: Some(format!("skills scan task failed: {join_err}")),
                    },
                });
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
/// The actor knows its scan path from injected context data,
/// so this command has no payload.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("skills")]
pub struct ScanSkills;

/// Spawns the skills scan actor on the given tokio runtime.
pub fn spawn_skills_scan_actor(
    scan_path: PathBuf,
    state: State,
    sink: Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
) -> (ActorRef<SkillsScanDirectMsg>, ActorSpawnResult) {
    let (tx, rx) = kanal::unbounded::<ActorEnvelope<SkillsScanDirectMsg>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new("skills-scan", sink);
    ctx.set_data(scan_path);
    ctx.set_data(state);
    let actor = SkillsScanActor::activate(&mut ctx);
    let result = spawn_actor("skills-scan", actor, &actor_ref, rx, ctx, handle);
    (actor_ref, result)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::common::actor::{ActorContext, ActorEnvelope, RecordingSink};
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::protocol::Command;

    use super::*;

    fn find_skills_loaded(events: &[Event]) -> Option<&SkillsLoaded> {
        for evt in events {
            if let Event::SkillsLoaded { payload } = evt {
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
        let skill_dir = dir.path().join("test-skill");
        std::fs::create_dir_all(&skill_dir).expect("create skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: test-skill\ndescription: A test skill\n---\n\n# Content",
        )
        .expect("write SKILL.md");

        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("skills-scan-test", sink.clone() as Arc<dyn MessageSink>);
        let state = State::new(AppState::default());
        ctx.set_data(dir.path().to_owned());
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
        ctx.set_data(dir.path().to_owned());
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
        ctx.set_data(dir.path().to_owned());
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
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("skills-scan-test", sink.clone() as Arc<dyn MessageSink>);
        let state = State::new(AppState::default());
        ctx.set_data(PathBuf::from("/nonexistent/skills"));
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
