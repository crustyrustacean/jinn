//! The export actor — writes a session to one self-contained file.
//!
//! Handles [`ExportSessionToFile`]. The handler captures a
//! [`SessionSnapshot`] of the target session under the state **read** lock,
//! drops the guard, and only then resolves the path, renders, and writes —
//! all of it inside `tokio::task::spawn_blocking`.
//!
//! The ordering is load-bearing rather than tidy. Path resolution stats the
//! filesystem (to climb the default-name ladder) and rendering walks every
//! entry including base64-encoding any attached images; doing either under
//! the lock would block the render thread for the length of a large session.
//! The `spawn_blocking` closure owns its snapshot and its arguments outright —
//! it cannot borrow the guard, and must not.
//!
//! The outcome is always visible in the chat: success pushes a transient
//! notice naming the written file, and every failure — no exporter for the
//! extension, unknown session, unwritable directory — pushes an `Error` entry
//! naming the reason. Nothing is written when the export fails, and nothing is
//! reported from inside the blocking closure.

use std::path::Path;
use std::path::PathBuf;

use jinn_core_types::SessionId;
use jinn_core_types::chat_entry::ChatEntry;
use jinn_export_msg::ExportCompleted;
use jinn_export_msg::ExportSessionToFile;
use jinn_kernel::common::state::State;
use jinn_session_history_msg::PushChatEntry;
use jinn_session_state::ChatSessionState;
use jinn_session_state::snapshot::SessionSnapshot;
use tracing::warn;
use trouper::actor::ActorPath;
use trouper::actor::MsgHandler;
use trouper::actor::ServiceActor;
use trouper::context::MsgCtx;
use trouper::inbox::OverloadPolicy;
use trouper::registry::RegistryError;
use trouper::system::ActorSystem;

use crate::document::project_document;
use crate::format::format_for_path;
use crate::format::unsupported_extension_error;
use crate::path::resolve_export_path;

/// The export actor's static trouper path.
pub const EXPORT_PATH: &str = "export";

/// Dependencies for [`ExportActor`].
#[derive(Clone)]
pub struct ExportActorDeps {
    /// Shared application state, for snapshotting the session to export.
    pub state: State,
}

/// The export actor.
pub struct ExportActor {
    state: State,
}

impl ServiceActor for ExportActor {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "trait contract: start is never called (spawn uses start_with)"
    )]
    async fn start(
        _args: &trouper::json::Json,
    ) -> Result<Self, error_stack::Report<RegistryError>> {
        // Never called: the spawn helper injects the deps via `start_with`.
        Err(
            error_stack::IntoReport::into_report(RegistryError::InvalidSpec)
                .attach("ExportActor is spawned via start_with"),
        )
    }
}

impl ExportActor {
    /// Spawns the actor at its static trouper path and returns the path.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "port convention: spawn takes owned deps and clones into start_with"
    )]
    pub fn spawn(system: &ActorSystem, deps: ExportActorDeps) -> ActorPath {
        let path = ActorPath::new(EXPORT_PATH);
        trouper::builder::spawn_service_builder::<Self>(system)
            .at(path.clone())
            .start_with({
                let deps = deps.clone();
                move || {
                    let deps = deps.clone();
                    Box::pin(async move { Ok(Self { state: deps.state }) })
                }
            })
            .handles::<ExportSessionToFile>()
            // trouper enforces an emit gate: an outbound message that the
            // actor has not declared here is dead-lettered at flush and
            // never routes. Without these two lines the export writes its
            // file and then says nothing at all.
            .emits::<PushChatEntry>()
            .emits::<ExportCompleted>()
            .mailbox(16, OverloadPolicy::Block)
            .start();
        path
    }

    /// Handles an export request end to end.
    pub async fn handle_export(&self, msg: &ExportSessionToFile, ctx: &mut MsgCtx<'_>) {
        // Take the snapshot under the read lock, then release it. The block
        // bounds the guard's lifetime so nothing downstream can hold it
        // across the blocking work.
        let snapshot = {
            let state = self.state.read();
            state
                .session
                .get(&msg.session_id)
                .map(ChatSessionState::capture_snapshot)
        };

        let Some(snapshot) = snapshot else {
            Self::report_error(
                &msg.session_id,
                &format!(
                    "cannot export: no loaded session with id {}",
                    msg.session_id
                ),
                ctx,
            );
            return;
        };

        let result = self.render_and_write(msg, snapshot).await;

        match result {
            Ok(path) => {
                Self::report_written(&msg.session_id, &path, ctx);
                ctx.publish(ExportCompleted {
                    session_id: msg.session_id.clone(),
                    path,
                });
            }
            Err(reason) => {
                Self::report_error(&msg.session_id, &reason, ctx);
            }
        }
    }

    /// Resolves the path, renders the document, and writes the file.
    ///
    /// Everything here runs on a blocking worker: it stats the filesystem,
    /// walks every entry, and base64-encodes any attached images. The
    /// closure owns the snapshot and the requested path outright.
    async fn render_and_write(
        &self,
        msg: &ExportSessionToFile,
        snapshot: SessionSnapshot,
    ) -> Result<PathBuf, String> {
        let requested = msg.path.clone();
        let job = RenderJob {
            argument: requested.to_string_lossy().to_string(),
            cwd: snapshot.metadata.cwd.clone(),
            document: project_document(&snapshot),
        };
        tokio::task::spawn_blocking(move || render_and_write_blocking(&job))
            .await
            .unwrap_or_else(|e| Err(format!("export worker panicked: {e}")))
    }

    /// Pushes a transient entry naming the written file.
    ///
    /// Transient rather than `System`: the export result is a UI-only notice,
    /// never part of the conversation. A `System` entry would be persisted
    /// with the session and re-read on every load, and a second export of
    /// the same session would accumulate one more each time.
    fn report_written(session_id: &SessionId, path: &Path, ctx: &mut MsgCtx<'_>) {
        ctx.publish(PushChatEntry {
            session_id: session_id.clone(),
            entry: ChatEntry::transient(written_notice(path)),
            pin: None,
        });
    }

    /// Pushes an `Error` entry naming why the export failed.
    fn report_error(session_id: &SessionId, reason: &str, ctx: &mut MsgCtx<'_>) {
        warn!(%reason, "session export failed");
        ctx.publish(PushChatEntry {
            session_id: session_id.clone(),
            entry: ChatEntry::error(format!("Export failed: {reason}")),
            pin: None,
        });
    }
}

/// Everything the blocking write needs, owned so it can cross the thread
/// boundary without borrowing the state guard.
struct RenderJob {
    /// The destination, as typed.
    argument: String,
    /// The session's working directory, used to resolve a relative or
    /// default destination.
    cwd: PathBuf,
    /// The projected document.
    document: crate::document::ExportDocument,
}

/// The chat notice shown after a successful export.
///
/// Names the file, not the full path: the file was written beside the
/// session the user is already looking at, so the directory is redundant and
/// would wrap badly in a narrow terminal. The fallback covers a destination
/// that is a bare directory.
fn written_notice(path: &Path) -> String {
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    format!("Exported to {name}")
}

/// Renders and writes one export. Runs on a blocking worker.
fn render_and_write_blocking(job: &RenderJob) -> Result<PathBuf, String> {
    let path = resolve_export_path(&job.argument, &job.cwd).map_err(|e| e.reason)?;
    let Some(format) = format_for_path(&path) else {
        // Refused before any write: no partial file is left behind.
        return Err(unsupported_extension_error(&path).reason);
    };
    let rendered = format.render(&job.document);
    std::fs::write(&path, rendered).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

impl MsgHandler<ExportSessionToFile> for ExportActor {
    async fn handle(&mut self, msg: &ExportSessionToFile, ctx: &mut MsgCtx<'_>) {
        self.handle_export(msg, ctx).await;
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
    use super::*;
    use crate::document::ExportDocument;
    use crate::document::ExportEntry;
    use jiff::Timestamp;

    fn job(argument: &str, cwd: &std::path::Path) -> RenderJob {
        RenderJob {
            argument: argument.to_owned(),
            cwd: cwd.to_path_buf(),
            document: ExportDocument {
                title: "Smoke".to_owned(),
                model: "m".to_owned(),
                cwd: cwd.to_path_buf(),
                created_at: Timestamp::UNIX_EPOCH,
                updated_at: Timestamp::UNIX_EPOCH,
                session_id: "s".to_owned(),
                entries: vec![ExportEntry {
                    class: "assistant",
                    body: "## Heading".to_owned(),
                    summary: None,
                    is_disclosure: false,
                    images: Vec::new(),
                    tool_status: None,
                    line_count: None,
                    token_count: None,
                    timestamp: Timestamp::UNIX_EPOCH,
                }],
            },
        }
    }

    #[rstest::rstest]
    fn writes_an_html_file_to_the_typed_path() {
        // Given a job targeting a markdown-ineligible html path in a temp dir.
        let dir = tempfile::tempdir().expect("temp dir");

        // When running the blocking write.
        let path = render_and_write_blocking(&job("out.html", dir.path())).expect("write succeeds");

        // Then the file exists at that path and holds rendered HTML.
        assert_eq!(path, dir.path().join("out.html"));
        let written = std::fs::read_to_string(&path).expect("read back");
        assert!(written.starts_with("<!DOCTYPE html>"));
        assert!(written.contains("<h2>Heading</h2>"));
    }

    #[rstest::rstest]
    fn writes_a_markdown_file_when_the_extension_says_so() {
        // Given a job targeting a `.md` path.
        let dir = tempfile::tempdir().expect("temp dir");

        // When running the blocking write.
        let path = render_and_write_blocking(&job("out.md", dir.path())).expect("write succeeds");

        // Then markdown is written, not HTML.
        let written = std::fs::read_to_string(&path).expect("read back");
        assert!(written.starts_with("# Smoke"));
        assert!(!written.contains("<!DOCTYPE"));
    }

    #[rstest::rstest]
    fn unsupported_extension_writes_no_file() {
        // Given a job targeting a `.pdf` path.
        let dir = tempfile::tempdir().expect("temp dir");

        // When running the blocking write.
        let result = render_and_write_blocking(&job("out.pdf", dir.path()));

        // Then it is refused with the exporter wording, and nothing is written.
        let reason = result.expect_err("pdf is not a supported export format");
        assert!(reason.starts_with("no exporter for .pdf"), "was: {reason}");
        assert!(!dir.path().join("out.pdf").exists());
    }

    #[rstest::rstest]
    fn the_success_notice_names_the_written_file() {
        // Given a written path inside a session directory.
        let path = Path::new("/home/dev/project/chat-export.html");

        // When building the chat notice.
        let notice = written_notice(path);

        // Then it names the file, not the whole path.
        assert_eq!(notice, "Exported to chat-export.html");
    }

    #[rstest::rstest]
    fn the_success_notice_survives_a_directory_destination() {
        // Given a destination that names a directory.
        let path = Path::new("/home/dev/project");

        // When building the chat notice.
        let notice = written_notice(path);

        // Then it still renders a name rather than an empty one.
        assert_eq!(notice, "Exported to project");
    }

    #[rstest::rstest]
    fn a_text_file_is_refused_with_the_exporter_wording() {
        // Given a job targeting a `.txt` path.
        let dir = tempfile::tempdir().expect("temp dir");

        // When running the blocking write.
        let result = render_and_write_blocking(&job("chat.txt", dir.path()));

        // Then the user is told there is no exporter for it.
        let reason = result.expect_err("txt has no exporter");
        assert!(reason.starts_with("no exporter for .txt"), "was: {reason}");
        assert!(!dir.path().join("chat.txt").exists());
    }

    #[rstest::rstest]
    fn empty_argument_writes_the_default_filename() {
        // Given a job with no typed path.
        let dir = tempfile::tempdir().expect("temp dir");

        // When running the blocking write.
        let path = render_and_write_blocking(&job("", dir.path())).expect("write succeeds");

        // Then the default filename is used.
        assert_eq!(path, dir.path().join("chat-export.html"));
        assert!(path.exists());
    }

    #[rstest::rstest]
    fn unwritable_destination_reports_the_reason() {
        // Given a job targeting a path inside a file, not a directory.
        let dir = tempfile::tempdir().expect("temp dir");
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "i am a file").expect("write");

        // When running the blocking write.
        let result = render_and_write_blocking(&job("blocker/out.html", dir.path()));

        // Then the failure names the path that could not be written.
        let reason = result.expect_err("writing under a file must fail");
        assert!(reason.contains("blocker/out.html"), "was: {reason}");
    }
}

#[cfg(test)]
mod bus_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use jinn_kernel::AppState;
    use jinn_testutil::bus_harness::TestHarness;

    /// Spawns an actor over a session whose cwd is `dir`, and returns the
    /// harness plus that session's id.
    async fn actor_over(dir: &std::path::Path) -> (TestHarness, SessionId) {
        let mut app = AppState::default();
        let mut session = jinn_session_state::ChatSessionState::new();
        session.set_cwd(dir.to_path_buf());
        // The map keys by the session's own id, so the request must name
        // that id — exactly what the chat-input slice sends.
        let session_id = session.session_id().clone();
        app.session.insert(session);
        app.session.set_active(session_id.clone());
        let harness = TestHarness::new().await;
        let _ = ExportActor::spawn(
            harness.system(),
            ExportActorDeps {
                state: State::new(app),
            },
        );
        (harness, session_id)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn a_published_export_command_writes_a_file() {
        // Given a spawned export actor over a real bus, and a session whose
        // working directory is a temp dir.
        let dir = tempfile::tempdir().expect("temp dir");
        let (harness, session_id) = actor_over(dir.path()).await;
        let rec = harness.spawn_recorder::<PushChatEntry>().await;

        // When publishing an export request for that directory.
        let msg = ExportSessionToFile {
            session_id,
            path: dir.path().join("out.html"),
        };
        harness.publish(msg).await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        // Then the chat carries a notice naming the written file.
        let pushed: Vec<String> = rec.drain().into_iter().map(|e| e.entry.text()).collect();
        assert_eq!(pushed, vec!["Exported to out.html".to_owned()]);

        // Then the file appears (eventually) on disk.
        let target = dir.path().join("out.html");
        for _ in 0..200 {
            if target.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(target.exists(), "no file was written by the actor");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn an_unsupported_extension_reports_an_error_in_the_chat() {
        // Given a spawned actor over a temp-dir session.
        let dir = tempfile::tempdir().expect("temp dir");
        let (harness, session_id) = actor_over(dir.path()).await;
        let rec = harness.spawn_recorder::<PushChatEntry>().await;

        // When publishing a request for an extension with no exporter.
        harness
            .publish(ExportSessionToFile {
                session_id,
                path: dir.path().join("chat.txt"),
            })
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        // Then the chat carries an error naming the missing exporter.
        let pushed: Vec<String> = rec.drain().into_iter().map(|e| e.entry.text()).collect();
        assert_eq!(pushed.len(), 1, "expected one error, got {pushed:?}");
        assert!(
            pushed
                .first()
                .is_some_and(|t| t.contains("no exporter for .txt")),
            "was: {pushed:?}"
        );
        // And no file was created.
        assert!(!dir.path().join("chat.txt").exists());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn an_unknown_session_reports_an_error_in_the_chat() {
        // Given a spawned actor over a temp-dir session.
        let dir = tempfile::tempdir().expect("temp dir");
        let (harness, _) = actor_over(dir.path()).await;
        let rec = harness.spawn_recorder::<PushChatEntry>().await;

        // When publishing a request naming a session that is not loaded.
        harness
            .publish(ExportSessionToFile {
                session_id: SessionId::new(),
                path: dir.path().join("out.html"),
            })
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        // Then the chat says so rather than staying silent.
        let pushed: Vec<String> = rec.drain().into_iter().map(|e| e.entry.text()).collect();
        assert_eq!(pushed.len(), 1, "expected one error, got {pushed:?}");
        assert!(
            pushed
                .first()
                .is_some_and(|t| t.contains("no loaded session")),
            "was: {pushed:?}"
        );
    }
}
