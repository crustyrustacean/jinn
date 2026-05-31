// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Judge scan actor - scans and loads judge files on command.
//!
//! Subscribes to [`RescanJudges`] commands, scans the judges directory,
//! and emits [`JudgesLoaded`] events with the results.

use crate::common::actor::scan_actor::NoDirectMsg;
use crate::common::actor::{Actor, ActorContext, ActorEnvelope};
use crate::common::app_paths::AppPaths;
use crate::protocol::{Command, Event};

use super::loader::scan_judges_merged;

/// Dependencies for [`JudgeScanActor`].
pub struct JudgeScanActorDeps {
    /// Application paths for resolving scan directories.
    pub paths: AppPaths,
}

/// Scans and loads judge files on `RescanJudges`.
///
/// On command, scans user and system judge directories, parses all
/// `*.md` files, and emits `JudgesLoaded` with the results.
pub struct JudgeScanActor {
    /// Application paths for resolving scan directories.
    paths: AppPaths,
}

impl Actor for JudgeScanActor {
    type Message = NoDirectMsg;
    type Deps = JudgeScanActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description("Scans and loads judge files from ~/.config/jinn/judges");
        ctx.subscribe_command::<super::protocol::RescanJudges>();
        Self { paths: deps.paths }
    }

    async fn handle(&mut self, msg: ActorEnvelope<NoDirectMsg>, ctx: &ActorContext) {
        if let ActorEnvelope::Command(command) = msg {
            self.handle_command(&command, ctx).await;
        }
    }
}

impl JudgeScanActor {
    /// Dispatches incoming commands.
    async fn handle_command(&mut self, command: &Command, ctx: &ActorContext) {
        if matches!(command, Command::RescanJudges(..)) {
            self.run_scan(ctx).await;
        }
    }

    /// Runs the blocking scan and emits the result.
    async fn run_scan(&self, ctx: &ActorContext) {
        let paths = self.paths.clone();
        let result = tokio::task::spawn_blocking(move || {
            scan_judges_merged(&paths.judges_dir(), &paths.system_judges_dir())
        })
        .await;

        match result {
            Ok(judges) => {
                tracing::info!(count = judges.len(), "rescanned judges");
                let _ = ctx.send_event(Event::JudgesLoaded(super::protocol::JudgesLoaded {
                    judges,
                    error: None,
                }));
            }
            Err(join_error) => {
                tracing::error!("judge rescan task panicked: {join_error}");
                let _ = ctx.send_event(Event::JudgesLoaded(super::protocol::JudgesLoaded {
                    judges: vec![],
                    error: Some(format!("rescan task failed: {join_error}")),
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use std::sync::Arc;

    use crate::common::actor::{Actor, ActorContext, ActorEnvelope, MessageSink, RecordingSink};
    use crate::common::app_paths::AppPaths;
    use crate::feat::judge::protocol;
    use crate::protocol::{Command, Event};

    use super::*;

    fn create_actor(dir: &tempfile::TempDir) -> (JudgeScanActor, Arc<RecordingSink>, ActorContext) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("judge-scan-test", sink.clone() as Arc<dyn MessageSink>);
        let deps = JudgeScanActorDeps {
            paths: AppPaths::new_in(dir.path()),
        };
        let actor = JudgeScanActor::activate(deps, &mut ctx);
        (actor, sink, ctx)
    }

    fn find_judges_loaded(events: &[Event]) -> Option<&protocol::JudgesLoaded> {
        for evt in events {
            if let Event::JudgesLoaded(payload) = evt {
                return Some(payload);
            }
        }
        None
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_judges_command_emits_judges_loaded() {
        // Given an actor with a temp directory.
        let dir = tempfile::tempdir().expect("create temp dir");
        let (mut actor, sink, ctx) = create_actor(&dir);

        // When processing RescanJudges command.
        actor
            .handle(
                ActorEnvelope::Command(Command::RescanJudges(protocol::RescanJudges)),
                &ctx,
            )
            .await;

        // Then JudgesLoaded event is emitted.
        let events = sink.events();
        let loaded = find_judges_loaded(&events);
        assert!(loaded.is_some());
        let loaded = loaded.expect("should have JudgesLoaded");
        assert!(loaded.error.is_none());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_judges_empty_dir_emits_empty_loaded() {
        // Given an actor with an empty temp directory.
        let dir = tempfile::tempdir().expect("create temp dir");
        let (mut actor, sink, ctx) = create_actor(&dir);

        // When processing RescanJudges command.
        actor
            .handle(
                ActorEnvelope::Command(Command::RescanJudges(protocol::RescanJudges)),
                &ctx,
            )
            .await;

        // Then JudgesLoaded has empty list.
        let events = sink.events();
        let loaded = find_judges_loaded(&events).expect("should have JudgesLoaded");
        assert!(loaded.judges.is_empty());
        assert!(loaded.error.is_none());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn scan_judges_finds_judge_files() {
        // Given an actor with a judge file in the user judges dir.
        let dir = tempfile::tempdir().expect("create temp dir");
        let judges_dir = dir.path().join("config/jinn/judges");
        std::fs::create_dir_all(&judges_dir).expect("create dir");
        std::fs::write(
            judges_dir.join("accuracy.md"),
            "+++\nname = \"accuracy\"\ndescription = \"Checks\"\n+++\n\nBody.",
        )
        .expect("write");

        let (mut actor, sink, ctx) = create_actor(&dir);

        // When processing RescanJudges command.
        actor
            .handle(
                ActorEnvelope::Command(Command::RescanJudges(protocol::RescanJudges)),
                &ctx,
            )
            .await;

        // Then JudgesLoaded has the judge.
        let events = sink.events();
        let loaded = find_judges_loaded(&events).expect("should have JudgesLoaded");
        assert_eq!(loaded.judges.len(), 1);
        assert_eq!(loaded.judges[0].name, "accuracy");
    }
}
