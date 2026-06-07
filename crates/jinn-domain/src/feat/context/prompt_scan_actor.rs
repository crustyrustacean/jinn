//! Prompt template scan actor - scans and reloads prompt templates on command.
//!
//! Subscribes to [`RescanPromptTemplates`] commands, scans system, user, and
//! project prompts directories for the active session's cwd, writes the merged
//! result into that session's ephemeral discovered-prompt-templates store, and
//! emits [`PromptTemplatesLoaded`] events with the results.

use crate::common::actor::scan_actor::NoDirectMsg;
use crate::common::actor::{Actor, ActorContext, ActorEnvelope};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::context::prompt_template::PromptTemplateStore;
use crate::feat::discovery::project_prompts_dirs;
use crate::feat::provider::protocol::command::RescanPromptTemplates;
use crate::feat::provider::protocol::event::PromptTemplatesLoaded;
use crate::protocol::{Command, Event};

/// Dependencies for [`PromptScanActor`].
pub struct PromptScanActorDeps {
    /// Runtime services.
    pub services: Services,
    /// Shared application state.
    pub state: State,
}

/// Scans and reloads prompt templates on `RescanPromptTemplates`.
///
/// On command, reads the session's cwd from shared state, scans system, user,
/// and project prompts directories (project templates override system/user on a
/// most-local-wins basis), writes the merged store into that session's
/// ephemeral discovered set, and emits `PromptTemplatesLoaded`.
pub struct PromptScanActor {
    /// Runtime services.
    services: Services,
    /// Shared application state.
    state: State,
}

impl Actor for PromptScanActor {
    type Message = NoDirectMsg;
    type Deps = PromptScanActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description("Scans and reloads prompt templates");
        ctx.subscribe_command::<RescanPromptTemplates>();
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

impl PromptScanActor {
    /// Dispatches incoming commands.
    async fn handle_command(&mut self, command: &Command, ctx: &ActorContext) {
        if let Command::RescanPromptTemplates(payload) = command {
            self.run_scan(&payload.session_id, ctx).await;
        }
    }

    /// Runs the blocking scan for a session's cwd and emits the result.
    async fn run_scan(&self, session_id: &crate::SessionId, ctx: &ActorContext) {
        // Resolve the session's cwd and home once, up front. The cwd is
        // captured by clone so the blocking scan can move it across the
        // thread boundary without holding the state lock.
        let Some((cwd, home, user_dir, system_dir)) = self.resolve_scan_inputs(session_id)
        else {
            tracing::warn!(%session_id, "RescanPromptTemplates: session not found, skipping");
            return;
        };

        let project_dirs = project_prompts_dirs(&cwd, &home);

        let result = tokio::task::spawn_blocking(move || {
            PromptTemplateStore::load_from_dirs_ordered(&user_dir, &system_dir, &project_dirs)
        })
        .await;

        match result {
            Ok(Ok(store)) => {
                tracing::info!(count = store.len(), "rescanned prompt templates");

                {
                    let mut guard = self.state.write();
                    if let Some(session) = guard.try_session_mut(session_id) {
                        session.set_discovered_prompt_templates(store.clone());
                    }
                }

                let _ = ctx.send_event(Event::PromptTemplatesLoaded(PromptTemplatesLoaded {
                    session_id: session_id.clone(),
                    templates: store.templates().to_vec(),
                    error: None,
                }));
            }
            Ok(Err(e)) => {
                tracing::warn!("failed to rescan prompt templates: {e:?}");
                let _ = ctx.send_event(Event::PromptTemplatesLoaded(PromptTemplatesLoaded {
                    session_id: session_id.clone(),
                    templates: vec![],
                    error: Some(format!("{e:?}")),
                }));
            }
            Err(join_error) => {
                tracing::error!("rescan task panicked: {join_error}");
                let _ = ctx.send_event(Event::PromptTemplatesLoaded(PromptTemplatesLoaded {
                    session_id: session_id.clone(),
                    templates: vec![],
                    error: Some(format!("rescan task failed: {join_error}")),
                }));
            }
        }
    }

    /// Reads the session's cwd and the prompt dirs for the scan.
    ///
    /// Returns `None` if the session is not present in state (it may have been
    /// closed concurrently). All values are cheap clones that can move into a
    /// `spawn_blocking` closure.
    fn resolve_scan_inputs(
        &self,
        session_id: &crate::SessionId,
    ) -> Option<(
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
    )> {
        let guard = self.state.read();
        let session = guard.try_session(session_id)?;
        let cwd = session.cwd().to_path_buf();
        let home = self.services.paths.home_dir().to_path_buf();
        let user_dir = self.services.paths.prompts_dir();
        let system_dir = self.services.paths.system_prompts_dir();
        Some((cwd, home, user_dir, system_dir))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use std::sync::Arc;

    use crate::common::actor::{Actor, ActorContext, ActorEnvelope, MessageSink, RecordingSink};
    use crate::common::app_paths::AppPaths;
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::provider::protocol::{command::RescanPromptTemplates, event::PromptTemplatesLoaded};
    use crate::protocol::{Command, Event};

    use super::*;

    fn find_loaded(events: &[Event]) -> Option<&PromptTemplatesLoaded> {
        for evt in events {
            if let Event::PromptTemplatesLoaded(p) = evt {
                return Some(p);
            }
        }
        None
    }

    /// Build an actor whose active session has its cwd set to `dir`.
    fn create_actor(
        cwd: &std::path::Path,
        home: &tempfile::TempDir,
        state: State,
    ) -> (
        PromptScanActor,
        Arc<RecordingSink>,
        ActorContext,
        crate::SessionId,
    ) {
        {
            let mut guard = state.write();
            guard.session.active_session_mut().set_cwd(cwd.to_path_buf());
        }
        let session_id = state.read().session.active_session_id().clone();
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("prompt-scan-test", sink.clone() as Arc<dyn MessageSink>);
        let mut paths = AppPaths::new_in(home.path());
        paths.set_home_dir_for_test(home.path().to_path_buf());
        let services = crate::common::services::test_services::TestServices::builder()
            .paths(paths)
            .build();
        let deps = PromptScanActorDeps { services, state };
        let actor = PromptScanActor::activate(deps, &mut ctx);
        (actor, sink, ctx, session_id)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_prompts_writes_to_session_and_emits_session_tagged_event() {
        // Given a project prompt in a cwd that is a descendant of home.
        // (The walk is bounded by $HOME exclusive, so cwd must be inside home.)
        let dir = tempfile::tempdir().expect("create temp dir");
        let cwd = dir.path().join("work");
        let prompts_dir = cwd.join(".agents/prompts");
        std::fs::create_dir_all(&prompts_dir).expect("create prompts dir");
        std::fs::write(
            prompts_dir.join("code.md"),
            "+++\nname = \"code\"\ndescription = \"\"\n+++\nYou are a coder.",
        )
        .expect("write prompt");

        let state = State::new(AppState::default());
        let (mut actor, sink, ctx, session_id) = create_actor(&cwd, &dir, state.clone());

        // When scanning.
        actor
            .handle(
                ActorEnvelope::Command(Command::RescanPromptTemplates(RescanPromptTemplates {
                    session_id: session_id.clone(),
                })),
                &ctx,
            )
            .await;

        // Then the project prompt is in the session's discovered store.
        let guard = state.read();
        let session = guard
            .session
            .get(&session_id)
            .expect("session exists");
        let names: Vec<&str> = session
            .discovered_prompt_templates()
            .templates()
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert!(names.contains(&"code"), "project prompt discovered: {names:?}");

        // And the emitted event is tagged with the same session id.
        let events = sink.events();
        let loaded = find_loaded(&events).expect("should have PromptTemplatesLoaded");
        assert_eq!(loaded.session_id, session_id);
        assert!(loaded.error.is_none());
    }

    #[tokio::test]

    async fn scan_prompts_discovers_ancestor_template_from_nested_cwd() {
        // Given a prompt at home/repo/.agents/prompts but cwd is home/repo/subdir.
        let dir = tempfile::tempdir().expect("create temp dir");
        let repo = dir.path().join("repo");
        let subdir = repo.join("subdir");
        std::fs::create_dir_all(&subdir).expect("create nested dirs");
        let ancestor_prompts = repo.join(".agents/prompts");
        std::fs::create_dir_all(&ancestor_prompts).expect("create ancestor prompts dir");
        std::fs::write(
            ancestor_prompts.join("ancestor.md"),
            "+++\nname = \"ancestor\"\ndescription = \"\"\n+++\nYou are an ancestor prompt.",
        )
        .expect("write ancestor prompt");

        // And the session cwd is the nested subdir, home bounds the walk so the
        // ancestor repo layer is in scope.
        let state = State::new(AppState::default());
        let (mut actor, _sink, ctx, session_id) = create_actor(&subdir, &dir, state.clone());

        // When scanning.
        actor
            .handle(
                ActorEnvelope::Command(Command::RescanPromptTemplates(RescanPromptTemplates {
                    session_id: session_id.clone(),
                })),
                &ctx,
            )
            .await;

        // Then the ancestor prompt is discovered from the nested cwd.
        let guard = state.read();
        let session = guard
            .session
            .get(&session_id)
            .expect("session exists");
        let names: Vec<&str> = session
            .discovered_prompt_templates()
            .templates()
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert!(
            names.contains(&"ancestor"),
            "ancestor prompt discovered from nested cwd: {names:?}"
        );
    }
}
