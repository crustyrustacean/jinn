//! Skills scan actor - scans and loads agent skills.
//!
//! Two trigger paths:
//! - **Event-driven** (automatic): subscribes to session lifecycle events
//!   ([`EnvironmentLoaded`], [`SessionCreated`], [`SessionSetupCompleted`],
//!   [`SessionLoadCompleted`], [`SessionCwdChanged`]). Each event resolves a
//!   session id, applies the `"."`-sentinel gate via
//!   [`scan_cwd_for_session`](crate::common::actor::scan_actor::scan_cwd_for_session),
//!   and scans when the cwd is settled.
//! - **Command-driven** (manual reload): subscribes to
//!   [`ScanSkills`](crate::protocol::Command::ScanSkills) commands.
//!
//! On either trigger, scans the skills directory on a blocking thread, writes
//!   results to shared [`State`](crate::common::state::State), and emits
//!   [`SkillsLoaded`](crate::protocol::Event::SkillsLoaded) events.

use serde::{Deserialize, Serialize};

use crate::common::actor::scan_actor::{NoDirectMsg, scan_cwd_for_session};
use crate::common::actor::{Actor, ActorContext, ActorEnvelope};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::feat::session_lifecycle::protocol::event::{
    SessionCreated, SessionCwdChanged, SessionSetupCompleted,
};
use crate::feat::skills::scan::scan_skills_merged;
use crate::feat::skills::skill::Skill;
use crate::init::env_init_actor::EnvironmentLoaded;
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
/// writes results to the active session's ephemeral discovered state, and emits
/// `SkillsLoaded`.
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
        // Event-driven triggers: scan automatically when a session's cwd
        // becomes the active discovery target.
        ctx.subscribe_event::<EnvironmentLoaded>();
        ctx.subscribe_event::<SessionCreated>();
        ctx.subscribe_event::<SessionSetupCompleted>();
        ctx.subscribe_event::<SessionLoadCompleted>();
        ctx.subscribe_event::<SessionCwdChanged>();
        Self {
            services: deps.services,
            state: deps.state,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<NoDirectMsg>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(command) => {
                self.handle_command(&command, ctx).await;
            }
            ActorEnvelope::Event(event) => {
                self.handle_event(&event, ctx).await;
            }
            _ => {}
        }
    }
}

impl SkillsScanActor {
    /// Dispatches incoming commands.
    async fn handle_command(&mut self, command: &Command, ctx: &ActorContext) {
        if let Command::ScanSkills(payload) = command {
            self.run_scan(&payload.session_id, ctx).await;
        }
    }

    /// Dispatches incoming lifecycle events to a session-targeted scan.
    ///
    /// Extracts the relevant session id, applies the `"."`-sentinel gate via
    /// [`scan_cwd_for_session`], and scans when the cwd is settled. The gate
    /// defers lifecycle-setup sessions to `SessionSetupCompleted`.
    async fn handle_event(&self, event: &Event, ctx: &ActorContext) {
        let Some(session_id) = self.session_id_for_event(event) else {
            return;
        };
        if scan_cwd_for_session(&self.state, &session_id).is_some() {
            self.run_scan(&session_id, ctx).await;
        }
    }

    /// Resolves the session id a discovery trigger event targets, if any.
    fn session_id_for_event(&self, event: &Event) -> Option<crate::SessionId> {
        match event {
            Event::EnvironmentLoaded(_) => {
                Some(self.state.read().session.active_session_id().clone())
            }
            Event::SessionCreated(payload) => Some(payload.session_id.clone()),
            Event::SessionSetupCompleted(payload) => Some(payload.session_id.clone()),
            Event::SessionLoadCompleted(payload) => Some(payload.session_id().clone()),
            Event::SessionCwdChanged(payload) => Some(payload.session_id.clone()),
            _ => None,
        }
    }

    /// Runs the blocking scan for a session's cwd and emits the result.
    async fn run_scan(&self, session_id: &crate::SessionId, ctx: &ActorContext) {
        // Resolve the session's cwd and home once, up front. The cwd is
        // captured by clone so the blocking scan can move it across the
        // thread boundary without holding the state lock.
        let Some((cwd, home, global_skills_dir)) = self.resolve_scan_inputs(session_id) else {
            tracing::warn!(%session_id, "ScanSkills: session not found, skipping");
            return;
        };

        let project_dirs = crate::feat::discovery::project_skills_dirs(&cwd, &home);

        let result = tokio::task::spawn_blocking(move || {
            scan_skills_merged(&global_skills_dir, &project_dirs)
        })
        .await;

        match result {
            Ok(skills) => {
                tracing::info!(count = skills.len(), "scanned agent skills");

                // Write skills to the session's ephemeral discovered set and
                // reload picker entries from the active session.
                {
                    let mut guard = self.state.write();
                    if let Some(session) = guard.try_session_mut(session_id) {
                        session.set_discovered_skills(skills.clone());
                    }
                    // A rescan may discover changed bodies on disk; clear rendered
                    // previews so stale markdown is never redisplayed.
                    guard.frontend.caches.skill_preview_cache.write().clear();
                    super::reload::reload_skill_picker_entries(&mut guard);
                }

                let _ = ctx.send_event(Event::SkillsLoaded(SkillsLoaded {
                    session_id: session_id.clone(),
                    skills,
                    error: None,
                }));
            }
            Err(join_error) => {
                tracing::error!("skills scan task panicked: {join_error}");
                let _ = ctx.send_event(Event::SkillsLoaded(SkillsLoaded {
                    session_id: session_id.clone(),
                    skills: vec![],
                    error: Some(format!("skills scan task failed: {join_error}")),
                }));
            }
        }
    }

    /// Reads the session's cwd and the user's home dir for the scan.
    ///
    /// Returns `None` if the session is not present in state (it may have been
    /// closed concurrently). All three values are cheap clones that can move
    /// into a `spawn_blocking` closure.
    fn resolve_scan_inputs(
        &self,
        session_id: &crate::SessionId,
    ) -> Option<(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf)> {
        let guard = self.state.read();
        let session = guard.try_session(session_id)?;
        let cwd = session.cwd().to_path_buf();
        let home = self.services.paths.home_dir().to_path_buf();
        let global = self.services.paths.skills_dir();
        Some((cwd, home, global))
    }
}

/// Emitted when skills have been scanned and loaded.
///
/// On success, `skills` contains the discovered skills and `error` is `None`.
/// On failure, `skills` is empty and `error` contains a description.
#[derive(Debug, Clone, Serialize, Deserialize, EventMsg)]
#[event_msg("skills")]
pub struct SkillsLoaded {
    /// The session whose cwd drove the scan.
    pub session_id: crate::SessionId,
    /// The discovered agent skills.
    pub skills: Vec<Skill>,
    /// Error message if scanning failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Command to trigger a skills scan for a specific session.
///
/// The actor reads the session's cwd from state, scans global + project
/// dirs discovered via the bounded walk, and writes the merged result into
/// that session's ephemeral discovered-skills set.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("skills")]
pub struct ScanSkills {
    /// The session whose cwd drives the scan.
    pub session_id: crate::SessionId,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unreachable, clippy::indexing_slicing, reason = "test code")]
    use std::sync::Arc;

    use crate::common::actor::{Actor, ActorContext, ActorEnvelope, MessageSink, RecordingSink};
    use crate::common::app_paths::AppPaths;
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::session_lifecycle::protocol::event::{
        SessionCreated, SessionCwdChanged, SessionSetupCompleted,
    };
    use crate::init::env_init_actor::EnvironmentLoaded;
    use crate::protocol::Command;
    use jinn_selection_widget::PreviewCache;

    use super::*;

    fn find_skills_loaded(events: &[Event]) -> Option<&SkillsLoaded> {
        for evt in events {
            if let Event::SkillsLoaded(payload) = evt {
                return Some(payload);
            }
        }
        None
    }

    /// Builds an actor whose active session has its cwd set to `dir`.
    /// Returns the session id so tests can emit `ScanSkills { session_id }`.
    fn create_actor(
        dir: &tempfile::TempDir,
        state: State,
    ) -> (
        SkillsScanActor,
        Arc<RecordingSink>,
        ActorContext,
        crate::SessionId,
    ) {
        // Set the active session's cwd so the actor resolves `dir`.
        {
            let mut guard = state.write();
            guard
                .session
                .active_session_mut()
                .set_cwd(dir.path().to_path_buf());
        }
        let session_id = state.read().session.active_session_id().clone();

        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("skills-scan-test", sink.clone() as Arc<dyn MessageSink>);
        let services = crate::common::services::test_services::TestServices::builder()
            .paths(AppPaths::new_in(dir.path()))
            .build();
        let deps = SkillsScanActorDeps { services, state };

        let actor = SkillsScanActor::activate(deps, &mut ctx);
        (actor, sink, ctx, session_id)
    }

    /// Creates a temp dir containing a single skill `test-skill`.
    fn skill_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("create temp dir");
        let skills_base = dir.path().join(".agents/skills/test-skill");
        std::fs::create_dir_all(&skills_base).expect("create skill dir");
        std::fs::write(
            skills_base.join("SKILL.md"),
            "---\nname: test-skill\ndescription: A test skill\n---\n\n# Content",
        )
        .expect("write SKILL.md");
        dir
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
        let (mut actor, _sink, ctx, session_id) = create_actor(&dir, state.clone());

        // When processing ScanSkills command.
        actor
            .handle(
                ActorEnvelope::Command(Command::ScanSkills(crate::feat::skills::ScanSkills {
                    session_id: session_id.clone(),
                })),
                &ctx,
            )
            .await;

        // Then skills are written to the session's ephemeral discovered set.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session exists");
        assert_eq!(session.discovered_skills().len(), 1);
        assert_eq!(session.discovered_skills()[0].name, "test-skill");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_created_event_scans_skills() {
        // Given an actor whose active session cwd contains a skill.
        let dir = skill_dir();
        let state = State::new(AppState::default());
        let (actor, _sink, ctx, session_id) = create_actor(&dir, state.clone());

        // When processing SessionCreated for that session.
        let event = Event::SessionCreated(SessionCreated {
            session_id: session_id.clone(),
        });
        actor.handle_event(&event, &ctx).await;

        // Then the skill is written to the session's discovered set.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session exists");
        assert_eq!(session.discovered_skills().len(), 1);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_created_event_skips_scan_when_cwd_is_sentinel() {
        // Given an actor whose active session cwd is the pending "." sentinel.
        let dir = skill_dir();
        let state = State::new(AppState::default());
        // Note: deliberately do NOT set a real cwd; default is ".".
        let session_id = state.read().session.active_session_id().clone();
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("skills-scan-test", sink.clone() as Arc<dyn MessageSink>);
        let services = crate::common::services::test_services::TestServices::builder()
            .paths(AppPaths::new_in(dir.path()))
            .build();
        let actor = SkillsScanActor::activate(
            SkillsScanActorDeps {
                services,
                state: state.clone(),
            },
            &mut ctx,
        );

        // When processing SessionCreated for the sentinel-cwd session.
        let event = Event::SessionCreated(SessionCreated {
            session_id: session_id.clone(),
        });
        actor.handle_event(&event, &ctx).await;

        // Then no scan runs: the discovered set stays empty.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session exists");
        assert!(session.discovered_skills().is_empty());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_setup_completed_event_scans_skills() {
        // Given an actor whose active session cwd contains a skill.
        let dir = skill_dir();
        let state = State::new(AppState::default());
        let (actor, _sink, ctx, session_id) = create_actor(&dir, state.clone());

        // When processing SessionSetupCompleted.
        let event = Event::SessionSetupCompleted(SessionSetupCompleted {
            session_id: session_id.clone(),
            cwd: dir.path().to_path_buf(),
            error: None,
        });
        actor.handle_event(&event, &ctx).await;

        // Then the skill is written to the session's discovered set.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session exists");
        assert_eq!(session.discovered_skills().len(), 1);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_cwd_changed_event_scans_skills() {
        // Given an actor whose active session cwd contains a skill.
        let dir = skill_dir();
        let state = State::new(AppState::default());
        let (actor, _sink, ctx, session_id) = create_actor(&dir, state.clone());

        // When processing SessionCwdChanged.
        let event = Event::SessionCwdChanged(SessionCwdChanged {
            session_id: session_id.clone(),
            cwd: dir.path().to_path_buf(),
        });
        actor.handle_event(&event, &ctx).await;

        // Then the skill is written to the session's discovered set.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session exists");
        assert_eq!(session.discovered_skills().len(), 1);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn environment_loaded_event_scans_active_session_skills() {
        // Given an actor whose active session cwd contains a skill.
        let dir = skill_dir();
        let state = State::new(AppState::default());
        let (actor, _sink, ctx, session_id) = create_actor(&dir, state.clone());

        // When processing EnvironmentLoaded.
        let event = Event::EnvironmentLoaded(EnvironmentLoaded {
            config: crate::ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            },
        });
        actor.handle_event(&event, &ctx).await;

        // Then the active session's skill is discovered.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session exists");
        assert_eq!(session.discovered_skills().len(), 1);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_skills_clears_skill_preview_cache() {
        // Given an actor whose state holds a populated preview cache (from a
        // previous picker session).
        let dir = tempfile::tempdir().expect("create temp dir");
        let state = State::new(AppState::default());
        {
            let guard = state.write();
            guard.frontend.caches.skill_preview_cache.write().insert(
                "stale-skill".to_owned(),
                80,
                vec![ratatui::text::Line::raw("stale")],
            );
        }
        let (mut actor, _sink, ctx, session_id) = create_actor(&dir, state.clone());

        // When processing a ScanSkills command (rescan).
        actor
            .handle(
                ActorEnvelope::Command(Command::ScanSkills(crate::feat::skills::ScanSkills {
                    session_id,
                })),
                &ctx,
            )
            .await;

        // Then the cache is cleared so rescanned bodies are re-rendered fresh.
        assert!(
            state
                .read()
                .frontend
                .caches
                .skill_preview_cache
                .read()
                .is_empty(),
            "rescan must clear the skill preview cache"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_skills_command_emits_skills_loaded() {
        // Given an actor with a temp directory.
        let dir = tempfile::tempdir().expect("create temp dir");
        let state = State::new(AppState::default());
        let (mut actor, sink, ctx, session_id) = create_actor(&dir, state);

        // When processing ScanSkills command.
        actor
            .handle(
                ActorEnvelope::Command(Command::ScanSkills(crate::feat::skills::ScanSkills {
                    session_id,
                })),
                &ctx,
            )
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
        let (mut actor, sink, ctx, session_id) = create_actor(&dir, state);

        // When processing ScanSkills command.
        actor
            .handle(
                ActorEnvelope::Command(Command::ScanSkills(crate::feat::skills::ScanSkills {
                    session_id,
                })),
                &ctx,
            )
            .await;

        // Then SkillsLoaded has empty skills list.
        let events = sink.events();
        let loaded = find_skills_loaded(&events).expect("should have SkillsLoaded");
        assert!(loaded.skills.is_empty());
        assert!(loaded.error.is_none());
    }

    #[tokio::test]
    async fn scan_skills_replacing_cwd_clears_previous_discovered_skills() {
        // Given a session whose cwd has a project skill, scanned once so the
        // discovered set contains it.
        let home = tempfile::tempdir().expect("create home dir");
        let dir_with_skill = home.path().join("populated");
        std::fs::create_dir_all(&dir_with_skill).expect("create populated dir");
        let skill_dir = dir_with_skill.join(".agents").join("skills").join("alpha");
        std::fs::create_dir_all(&skill_dir).expect("create project skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: alpha\ndescription: alpha skill\n---\n\n# A",
        )
        .expect("write project SKILL.md");

        let empty_dir = home.path().join("empty");
        std::fs::create_dir_all(&empty_dir).expect("create empty dir");

        let paths_root = tempfile::tempdir().expect("create temp dir for AppPaths");
        let state = State::new(AppState::default());
        let session_id = state.read().session.active_session_id().clone();
        {
            let mut guard = state.write();
            guard
                .session
                .active_session_mut()
                .set_cwd(dir_with_skill.clone());
        }

        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("skills-scan-test", sink.clone() as Arc<dyn MessageSink>);
        let mut paths = AppPaths::new_in(paths_root.path());
        paths.set_home_dir_for_test(home.path().to_path_buf());
        let services = crate::common::services::test_services::TestServices::builder()
            .paths(paths)
            .build();
        let deps = SkillsScanActorDeps {
            services,
            state: state.clone(),
        };
        let mut actor = SkillsScanActor::activate(deps, &mut ctx);

        // First scan: discovers `alpha`.
        actor
            .handle(
                ActorEnvelope::Command(Command::ScanSkills(crate::feat::skills::ScanSkills {
                    session_id: session_id.clone(),
                })),
                &ctx,
            )
            .await;
        {
            let guard = state.read();
            let session = guard
                .session
                .get(&session_id)
                .expect("session exists after first scan");
            let skills = session.discovered_skills();
            assert_eq!(skills.len(), 1, "populated cwd yields one skill");
            assert_eq!(skills[0].name, "alpha");
        }

        // When the cwd changes to an empty dir and a second scan runs.
        {
            let mut guard = state.write();
            guard
                .session
                .active_session_mut()
                .set_cwd(empty_dir.clone());
        }
        actor
            .handle(
                ActorEnvelope::Command(Command::ScanSkills(crate::feat::skills::ScanSkills {
                    session_id: session_id.clone(),
                })),
                &ctx,
            )
            .await;

        // Then the discovered set is empty — no stale `alpha` carryover.
        let guard = state.read();
        let session = guard
            .session
            .get(&session_id)
            .expect("session exists after second scan");
        assert!(
            session.discovered_skills().is_empty(),
            "empty cwd must clear previously discovered skills"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_skills_nonexistent_dir_emits_empty_loaded() {
        // Given an actor with a nonexistent directory.
        let dir = tempfile::tempdir().expect("create temp dir");
        let state = State::new(AppState::default());
        let (mut actor, sink, ctx, session_id) = create_actor(&dir, state);

        // When processing ScanSkills command.
        actor
            .handle(
                ActorEnvelope::Command(Command::ScanSkills(crate::feat::skills::ScanSkills {
                    session_id,
                })),
                &ctx,
            )
            .await;

        // Then SkillsLoaded has empty skills list.
        let events = sink.events();
        let loaded = find_skills_loaded(&events).expect("should have SkillsLoaded");
        assert!(loaded.skills.is_empty());
        assert!(loaded.error.is_none());
    }

    /// Writes a skill `name` into a directory's `skills/<name>/SKILL.md`.
    fn write_skill(base: &std::path::Path, name: &str, body: &str) {
        let dir = base.join("skills").join(name);
        std::fs::create_dir_all(&dir).expect("create skill dir");
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {name} skill\n---\n\n{body}"),
        )
        .expect("write SKILL.md");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_skills_project_overrides_global_same_name() {
        // Given a global skill `shared` and a project skill `shared` at the cwd.
        let dir = tempfile::tempdir().expect("create temp dir");
        // Global skills live at dir/skills (AppPaths::new_in(dir)).
        write_skill(dir.path(), "shared", "# GLOBAL body");
        // Project skill lives at dir/.agents/skills/shared.
        let project_skill_dir = dir.path().join(".agents/skills/shared");
        std::fs::create_dir_all(&project_skill_dir).expect("create project skill dir");
        std::fs::write(
            project_skill_dir.join("SKILL.md"),
            "---\nname: shared\ndescription: shared skill\n---\n\n# PROJECT body",
        )
        .expect("write project SKILL.md");

        // And home is set ABOVE dir so the walk reaches the project layer.
        let state = State::new(AppState::default());
        let (mut actor, _sink, ctx, session_id) = create_actor(&dir, state.clone());

        // When scanning.
        actor
            .handle(
                ActorEnvelope::Command(Command::ScanSkills(crate::feat::skills::ScanSkills {
                    session_id: session_id.clone(),
                })),
                &ctx,
            )
            .await;

        // Then exactly one `shared` skill exists and it is the PROJECT one.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session exists");
        let skills = session.discovered_skills();
        assert_eq!(skills.len(), 1, "dedup to one `shared`");
        assert_eq!(skills[0].name, "shared");
        assert!(
            skills[0].body.contains("PROJECT body"),
            "project wins: {body}",
            body = skills[0].body
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_skills_discovers_ancestor_project_skill_from_nested_cwd() {
        // Given a tree home/repo/.agents/skills/ancestor and a session whose
        // cwd is home/repo/subdir (a descendant), with home set to home.
        // No VCS marker anywhere, so the walk is bounded by exclusive $HOME.
        let home = tempfile::tempdir().expect("create home dir");
        let repo = home.path().join("repo");
        let subdir = repo.join("subdir");
        std::fs::create_dir_all(&subdir).expect("create nested dirs");
        let ancestor_skill = repo.join(".agents/skills/ancestor/SKILL.md");
        std::fs::create_dir_all(ancestor_skill.parent().unwrap())
            .expect("create ancestor skill dir");
        std::fs::write(
            &ancestor_skill,
            "---\nname: ancestor\ndescription: ancestor skill\n---\n\n# ancestor body",
        )
        .expect("write ancestor skill");

        let dir = tempfile::tempdir().expect("create temp dir for AppPaths");
        let state = State::new(AppState::default());
        {
            let mut guard = state.write();
            guard.session.active_session_mut().set_cwd(subdir.clone());
        }
        let session_id = state.read().session.active_session_id().clone();

        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("skills-scan-test", sink.clone() as Arc<dyn MessageSink>);
        let mut paths = AppPaths::new_in(dir.path());
        paths.set_home_dir_for_test(home.path().to_path_buf());
        let services = crate::common::services::test_services::TestServices::builder()
            .paths(paths)
            .build();
        let deps = SkillsScanActorDeps {
            services,
            state: state.clone(),
        };
        let mut actor = SkillsScanActor::activate(deps, &mut ctx);

        // When scanning from the nested cwd.
        actor
            .handle(
                ActorEnvelope::Command(Command::ScanSkills(crate::feat::skills::ScanSkills {
                    session_id: session_id.clone(),
                })),
                &ctx,
            )
            .await;

        // Then the ancestor skill (one level up from cwd, within the bounded
        // walk) is discovered.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session exists");
        let skills = session.discovered_skills();
        assert_eq!(
            skills.len(),
            1,
            "expected only the ancestor skill, got {len}",
            len = skills.len()
        );
        assert_eq!(skills[0].name, "ancestor");
    }

    #[tokio::test]
    async fn scan_skills_routes_discovery_per_session_cwd() {
        // Two sessions with two different cwds, each with a distinct project skill.
        // Scanning each by its own session_id must populate that session only.
        let home = tempfile::tempdir().expect("create home dir");
        let dir_a = home.path().join("a");
        let dir_b = home.path().join("b");
        std::fs::create_dir_all(&dir_a).expect("create dir a");
        std::fs::create_dir_all(&dir_b).expect("create dir b");
        // dir_a has skill `alpha`, dir_b has skill `beta`.
        for (base, name, body) in [("a", "alpha", "# A"), ("b", "beta", "# B")] {
            let skill_dir = home
                .path()
                .join(base)
                .join(".agents")
                .join("skills")
                .join(name);
            std::fs::create_dir_all(&skill_dir).expect("create skill dir");
            std::fs::write(
                skill_dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {name} skill\n---\n\n{body}"),
            )
            .expect("write SKILL.md");
        }

        let paths_root = tempfile::tempdir().expect("create temp dir for AppPaths");
        let state = State::new(AppState::default());
        // Session A: cwd = dir_a.
        let session_a = state.read().session.active_session_id().clone();
        {
            let mut guard = state.write();
            guard.session.active_session_mut().set_cwd(dir_a.clone());
        }
        // Session B: create + set cwd = dir_b.
        let session_b = crate::SessionId::new();
        {
            let mut guard = state.write();
            let s = guard.session.get_or_create(&session_b);
            s.set_cwd(dir_b.clone());
        }

        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("skills-scan-test", sink.clone() as Arc<dyn MessageSink>);
        let mut paths = AppPaths::new_in(paths_root.path());
        paths.set_home_dir_for_test(home.path().to_path_buf());
        let services = crate::common::services::test_services::TestServices::builder()
            .paths(paths)
            .build();
        let deps = SkillsScanActorDeps {
            services,
            state: state.clone(),
        };
        let mut actor = SkillsScanActor::activate(deps, &mut ctx);

        // Scan session A, then session B.
        for id in [&session_a, &session_b] {
            actor
                .handle(
                    ActorEnvelope::Command(Command::ScanSkills(crate::feat::skills::ScanSkills {
                        session_id: id.clone(),
                    })),
                    &ctx,
                )
                .await;
        }

        // Then each session sees only its own skill.
        let guard = state.read();
        let skills_a = guard
            .session
            .get(&session_a)
            .expect("session a")
            .discovered_skills();
        let skills_b = guard
            .session
            .get(&session_b)
            .expect("session b")
            .discovered_skills();
        assert_eq!(skills_a.len(), 1, "session A should see only alpha");
        assert_eq!(skills_a[0].name, "alpha");
        assert_eq!(skills_b.len(), 1, "session B should see only beta");
        assert_eq!(skills_b[0].name, "beta");
    }
}
