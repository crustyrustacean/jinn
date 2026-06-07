//! Context-files scan actor - scans project context files.
//!
//! Two trigger paths:
//! - **Event-driven** (automatic): subscribes to session lifecycle events
//!   ([`EnvironmentLoaded`], [`SessionCreated`], [`SessionSetupCompleted`],
//!   [`SessionLoadCompleted`], [`SessionCwdChanged`]). Each event resolves a
//!   session id, applies the `"."`-sentinel gate via
//!   [`scan_cwd_for_session`](crate::common::actor::scan_actor::scan_cwd_for_session),
//!   and scans when the cwd is settled.
//! - **Command-driven** (manual reload): subscribes to
//!   [`ScanContextFiles`] commands.
//!
//! On either trigger, walks the bounded ancestor chain for the session's cwd
//!   (stopping at an exclusive `$HOME` or an inclusive VCS root, whichever
//!   comes first), reads the first existing candidate (AGENTS.md / CLAUDE.md)
//!   per walked dir, writes the result into that session's ephemeral
//!   discovered set, and emits [`ContextFilesLoaded`] events.

use crate::common::actor::scan_actor::{NoDirectMsg, scan_cwd_for_session};
use crate::common::actor::{Actor, ActorContext, ActorEnvelope};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::context::env_context::ContextFile;
use crate::feat::context::protocol::command::ScanContextFiles;
use crate::feat::context::protocol::event::ContextFilesLoaded;
use crate::feat::discovery::project_context_files;
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::feat::session_lifecycle::protocol::event::{
    SessionCreated, SessionCwdChanged, SessionSetupCompleted,
};
use crate::init::env_init_actor::EnvironmentLoaded;
use crate::protocol::{Command, Event};

/// Dependencies for [`ContextFilesScanActor`].
pub struct ContextFilesScanActorDeps {
    /// Runtime services.
    pub services: Services,
    /// Shared application state.
    pub state: State,
}

/// Scans and loads project context files (AGENTS.md/CLAUDE.md) on `ScanContextFiles`.
///
/// On command, reads the session's cwd from shared state, walks the bounded
/// ancestor chain, reads each discovered context file, writes the result into
/// that session's ephemeral discovered set, and emits `ContextFilesLoaded`.
pub struct ContextFilesScanActor {
    /// Runtime services.
    services: Services,
    /// Shared application state.
    state: State,
}

impl Actor for ContextFilesScanActor {
    type Message = NoDirectMsg;
    type Deps = ContextFilesScanActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description("Scans and loads project context files (AGENTS.md/CLAUDE.md)");
        ctx.subscribe_command::<ScanContextFiles>();
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

impl ContextFilesScanActor {
    /// Dispatches incoming commands.
    async fn handle_command(&mut self, command: &Command, ctx: &ActorContext) {
        if let Command::ScanContextFiles(payload) = command {
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
        let Some((cwd, home)) = self.resolve_scan_inputs(session_id) else {
            tracing::warn!(%session_id, "ScanContextFiles: session not found, skipping");
            return;
        };

        let result = tokio::task::spawn_blocking(move || read_context_files(&cwd, &home)).await;

        match result {
            Ok(files) => {
                tracing::info!(count = files.len(), "scanned project context files");

                {
                    let mut guard = self.state.write();
                    if let Some(session) = guard.try_session_mut(session_id) {
                        session.set_discovered_context_files(files.clone());
                    }
                }

                let _ = ctx.send_event(Event::ContextFilesLoaded(ContextFilesLoaded {
                    session_id: session_id.clone(),
                    files,
                    error: None,
                }));
            }
            Err(join_error) => {
                tracing::error!("context-files scan task panicked: {join_error}");
                let _ = ctx.send_event(Event::ContextFilesLoaded(ContextFilesLoaded {
                    session_id: session_id.clone(),
                    files: vec![],
                    error: Some(format!("context-files scan task failed: {join_error}")),
                }));
            }
        }
    }

    /// Reads the session's cwd and the user's home dir for the scan.
    ///
    /// Returns `None` if the session is not present in state (it may have been
    /// closed concurrently). Both values are cheap clones that can move into a
    /// `spawn_blocking` closure.
    fn resolve_scan_inputs(
        &self,
        session_id: &crate::SessionId,
    ) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        let guard = self.state.read();
        let session = guard.try_session(session_id)?;
        let cwd = session.cwd().to_path_buf();
        let home = self.services.paths.home_dir().to_path_buf();
        Some((cwd, home))
    }
}

/// Reads the bounded-walk context files into `ContextFile` payloads.
///
/// Uses [`project_context_files`] to resolve candidate paths in bounded walk
/// order (least-local → cwd), then reads each file's contents. Files that
/// vanish between resolution and read are skipped silently.
fn read_context_files(cwd: &std::path::Path, home: &std::path::Path) -> Vec<ContextFile> {
    project_context_files(cwd, home)
        .into_iter()
        .filter_map(|path| read_one_context_file(&path))
        .collect()
}

/// Reads a single context file, returning `None` if it can no longer be read.
fn read_one_context_file(path: &std::path::Path) -> Option<ContextFile> {
    let content = std::fs::read_to_string(path).ok()?;
    Some(ContextFile {
        path: path.to_path_buf(),
        content,
    })
}

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

    fn find_context_files_loaded(events: &[Event]) -> Option<&ContextFilesLoaded> {
        for evt in events {
            if let Event::ContextFilesLoaded(payload) = evt {
                return Some(payload);
            }
        }
        None
    }

    /// Builds an actor whose active session has its cwd set to `dir`.
    /// Returns the session id so tests can emit `ScanContextFiles { session_id }`.
    fn create_actor(
        dir: &tempfile::TempDir,
        state: State,
    ) -> (
        ContextFilesScanActor,
        Arc<RecordingSink>,
        ActorContext,
        crate::SessionId,
    ) {
        {
            let mut guard = state.write();
            guard
                .session
                .active_session_mut()
                .set_cwd(dir.path().to_path_buf());
        }
        let session_id = state.read().session.active_session_id().clone();

        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new(
            "context-files-scan-test",
            sink.clone() as Arc<dyn MessageSink>,
        );
        // Set the home directory to a parent of `dir` so the project walk (which
        // is exclusive of `$HOME`) still visits the session's cwd and its ancestors.
        let home = dir
            .path()
            .parent()
            .expect("temp dir has a parent")
            .to_path_buf();
        let services = crate::common::services::test_services::TestServices::builder()
            .paths({
                let mut paths = AppPaths::new_in(dir.path());
                paths.set_home_dir_for_test(home);
                paths
            })
            .build();

        let deps = ContextFilesScanActorDeps { services, state };

        let actor = ContextFilesScanActor::activate(deps, &mut ctx);
        (actor, sink, ctx, session_id)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_context_files_command_writes_to_session_state() {
        // Given an actor whose session cwd has an AGENTS.md.
        let dir = tempfile::tempdir().expect("create temp dir");
        std::fs::write(dir.path().join("AGENTS.md"), "# Project rules").expect("write");

        let state = State::new(AppState::default());
        let (mut actor, _sink, ctx, session_id) = create_actor(&dir, state.clone());

        // When processing ScanContextFiles command.
        actor
            .handle(
                ActorEnvelope::Command(Command::ScanContextFiles(
                    crate::feat::context::protocol::command::ScanContextFiles {
                        session_id: session_id.clone(),
                    },
                )),
                &ctx,
            )
            .await;

        // Then the file is written to the session's ephemeral discovered set.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session exists");
        assert_eq!(session.discovered_context_files().len(), 1);
        assert_eq!(
            session.discovered_context_files()[0].content,
            "# Project rules"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_context_files_command_emits_loaded_event() {
        // Given an actor whose session cwd has an AGENTS.md.
        let dir = tempfile::tempdir().expect("create temp dir");
        std::fs::write(dir.path().join("AGENTS.md"), "# Body").expect("write");

        let state = State::new(AppState::default());
        let (mut actor, sink, ctx, session_id) = create_actor(&dir, state);

        // When processing ScanContextFiles command.
        actor
            .handle(
                ActorEnvelope::Command(Command::ScanContextFiles(
                    crate::feat::context::protocol::command::ScanContextFiles { session_id },
                )),
                &ctx,
            )
            .await;

        // Then ContextFilesLoaded is emitted with the file and no error.
        let events = sink.events();
        let loaded = find_context_files_loaded(&events).expect("should have ContextFilesLoaded");
        assert!(loaded.error.is_none());
        assert_eq!(loaded.files.len(), 1);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_context_files_empty_dir_emits_empty_loaded() {
        // Given an actor whose session cwd has no context files.
        let dir = tempfile::tempdir().expect("create temp dir");
        let state = State::new(AppState::default());
        let (mut actor, sink, ctx, session_id) = create_actor(&dir, state);

        // When processing ScanContextFiles command.
        actor
            .handle(
                ActorEnvelope::Command(Command::ScanContextFiles(
                    crate::feat::context::protocol::command::ScanContextFiles { session_id },
                )),
                &ctx,
            )
            .await;

        // Then ContextFilesLoaded is emitted with an empty list.
        let events = sink.events();
        let loaded = find_context_files_loaded(&events).expect("should have ContextFilesLoaded");
        assert!(loaded.files.is_empty());
        assert!(loaded.error.is_none());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_context_files_skips_home_ancestor() {
        // Given a session cwd nested under a "home" dir that itself has an AGENTS.md,
        // and no VCS marker anywhere - so the walk would reach home if unbounded.
        let home = tempfile::tempdir().expect("create home dir");
        std::fs::write(home.path().join("AGENTS.md"), "home-level file").expect("write home file");
        let project = home.path().join("repo");
        std::fs::create_dir_all(&project).expect("create project");
        std::fs::write(project.join("AGENTS.md"), "project file").expect("write project file");

        let dir = tempfile::tempdir().expect("create temp dir for AppPaths");
        let state = State::new(AppState::default());
        // Point the session cwd at the project dir, and home at the parent.
        {
            let mut guard = state.write();
            guard.session.active_session_mut().set_cwd(project.clone());
        }
        let session_id = state.read().session.active_session_id().clone();

        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new(
            "context-files-scan-test",
            sink.clone() as Arc<dyn MessageSink>,
        );
        // Build services whose home_dir() resolves to our temp home.
        let mut paths = AppPaths::new_in(dir.path());
        paths.set_home_dir_for_test(home.path().to_path_buf());
        let services = crate::common::services::test_services::TestServices::builder()
            .paths(paths)
            .build();
        let deps = ContextFilesScanActorDeps { services, state };
        let mut actor = ContextFilesScanActor::activate(deps, &mut ctx);

        // When processing ScanContextFiles command.
        actor
            .handle(
                ActorEnvelope::Command(Command::ScanContextFiles(
                    crate::feat::context::protocol::command::ScanContextFiles { session_id },
                )),
                &ctx,
            )
            .await;

        // Then only the project file is loaded; the home-level file is excluded.
        let events = sink.events();
        let loaded = find_context_files_loaded(&events).expect("should have ContextFilesLoaded");
        assert_eq!(loaded.files.len(), 1);
        assert_eq!(loaded.files[0].content, "project file");
    }

    #[tokio::test]
    async fn scan_context_files_discovers_ancestor_file_from_nested_cwd() {
        // Given cwd = home/repo/subdir and an AGENTS.md at home/repo (an ancestor
        // below home, no VCS marker). The walk must collect from `repo` even though
        // the session cwd is its descendant `subdir`.
        let home = tempfile::tempdir().expect("create home dir");
        let repo = home.path().join("repo");
        let subdir = repo.join("subdir");
        std::fs::create_dir_all(&subdir).expect("create nested dirs");
        std::fs::write(repo.join("AGENTS.md"), "ancestor file").expect("write ancestor file");

        let dir = tempfile::tempdir().expect("create temp dir for AppPaths");
        let state = State::new(AppState::default());
        {
            let mut guard = state.write();
            guard.session.active_session_mut().set_cwd(subdir.clone());
        }
        let session_id = state.read().session.active_session_id().clone();

        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new(
            "context-files-scan-test",
            sink.clone() as Arc<dyn MessageSink>,
        );
        let mut paths = AppPaths::new_in(dir.path());
        paths.set_home_dir_for_test(home.path().to_path_buf());
        let services = crate::common::services::test_services::TestServices::builder()
            .paths(paths)
            .build();
        let deps = ContextFilesScanActorDeps { services, state };
        let mut actor = ContextFilesScanActor::activate(deps, &mut ctx);

        actor
            .handle(
                ActorEnvelope::Command(Command::ScanContextFiles(
                    crate::feat::context::protocol::command::ScanContextFiles { session_id },
                )),
                &ctx,
            )
            .await;

        let events = sink.events();
        let loaded = find_context_files_loaded(&events).expect("should have ContextFilesLoaded");
        assert_eq!(
            loaded.files.len(),
            1,
            "ancestor file must be discovered from a descendant cwd"
        );
        assert_eq!(loaded.files[0].content, "ancestor file");
    }
}
