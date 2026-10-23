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

use error_stack::{IntoReport as _, Report};
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
use marked_path::{Absolute, MarkedPathAccess, MarkedPathBuf};

/// The export actor's static trouper path.
pub const EXPORT_PATH: &str = "export";

/// Why an export could not be written.
///
/// Variants carry everything the chat entry renders in their message: the
/// actor pushes the `Report`'s `Display` into the failure entry, and it
/// prints only the outermost context.
#[derive(Debug, wherror::Error)]
pub enum ExportWriteError {
    /// The destination argument could not be resolved to a path.
    #[error("cannot resolve export destination: {reason}")]
    ResolvePath {
        /// What the user can do about it, in one sentence.
        reason: String,
    },
    /// The destination's extension names no writer this slice has.
    #[error("{reason}")]
    UnsupportedExtension {
        /// A ready-to-render sentence naming the supported extensions.
        reason: String,
    },
    /// The rendered file could not be written.
    #[error("cannot write {path}: {detail}")]
    Write {
        /// The destination path.
        path: String,
        /// The filesystem's complaint.
        detail: String,
    },
    /// The blocking worker panicked before producing a result.
    #[error("export worker panicked: {detail}")]
    WorkerPanicked {
        /// The panic payload.
        detail: String,
    },
}

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
                Self::report_error(&msg.session_id, &reason.to_string(), ctx);
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
    ) -> Result<MarkedPathBuf<Absolute>, Report<ExportWriteError>> {
        let requested = msg.path.clone();
        let job = RenderJob {
            argument: requested.to_string(),
            cwd: snapshot.metadata.cwd.clone(),
            document: project_document(&snapshot),
        };
        tokio::task::spawn_blocking(move || render_and_write_blocking(&job))
            .await
            .map_err(|e| {
                ExportWriteError::WorkerPanicked {
                    detail: e.to_string(),
                }
                .into_report()
            })?
    }

    /// Pushes a transient entry naming the written file.
    ///
    /// Transient rather than `System`: the export result is a UI-only notice,
    /// never part of the conversation. A `System` entry would be persisted
    /// with the session and re-read on every load, and a second export of
    /// the same session would accumulate one more each time.
    fn report_written(
        session_id: &SessionId,
        path: &MarkedPathBuf<Absolute>,
        ctx: &mut MsgCtx<'_>,
    ) {
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
    cwd: MarkedPathBuf<Absolute>,
    /// The projected document.
    document: crate::document::ExportDocument,
}

/// The chat notice shown after a successful export.
///
/// Names the file, not the full path: the file was written beside the
/// session the user is already looking at, so the directory is redundant and
/// would wrap badly in a narrow terminal. The fallback covers a destination
/// that is a bare directory.
fn written_notice(path: &MarkedPathBuf<Absolute>) -> String {
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    format!("Exported to {name}")
}

/// Renders and writes one export. Runs on a blocking worker.
///
/// # Errors
///
/// Returns [`ExportWriteError`] naming the failed stage: path resolution,
/// an unwritable extension, or the write itself.
fn render_and_write_blocking(
    job: &RenderJob,
) -> Result<MarkedPathBuf<Absolute>, Report<ExportWriteError>> {
    let path = resolve_export_path(&job.argument, &job.cwd)
        .map_err(|e| Report::new(ExportWriteError::ResolvePath { reason: e.reason }))?;
    let Some(format) = format_for_path(&path) else {
        // Refused before any write: no partial file is left behind.
        let unsupported = unsupported_extension_error(&path);
        return Err(Report::new(ExportWriteError::UnsupportedExtension {
            reason: unsupported.reason,
        }));
    };
    let rendered = format.render(&job.document);
    std::fs::write(path.as_path(), rendered).map_err(|e| {
        ExportWriteError::Write {
            path: path.to_string(),
            detail: e.to_string(),
        }
        .into_report()
    })?;
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

    fn job(argument: &str, cwd: &MarkedPathBuf<Absolute>) -> RenderJob {
        RenderJob {
            argument: argument.to_owned(),
            cwd: cwd.clone(),
            document: ExportDocument {
                title: "Smoke".to_owned(),
                model: "m".to_owned(),
                cwd: cwd.clone(),
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
        let dir = jinn_testutil::tempdir();

        // When running the blocking write.
        let path = render_and_write_blocking(&job("out.html", &dir.path)).expect("write succeeds");

        // Then the file exists at that path and holds rendered HTML.
        assert_eq!(
            path,
            dir.path
                .join("out.html")
                .expect("temp root joins a bare name")
        );
        let written = std::fs::read_to_string(&path).expect("read back");
        assert!(written.starts_with("<!DOCTYPE html>"));
        assert!(written.contains("<h2>Heading</h2>"));
    }

    #[rstest::rstest]
    fn writes_a_markdown_file_when_the_extension_says_so() {
        // Given a job targeting a `.md` path.
        let dir = jinn_testutil::tempdir();

        // When running the blocking write.
        let path = render_and_write_blocking(&job("out.md", &dir.path)).expect("write succeeds");

        // Then markdown is written, not HTML.
        let written = std::fs::read_to_string(&path).expect("read back");
        assert!(written.starts_with("# Smoke"));
        assert!(!written.contains("<!DOCTYPE"));
    }

    #[rstest::rstest]
    fn unsupported_extension_writes_no_file() {
        // Given a job targeting a `.pdf` path.
        let dir = jinn_testutil::tempdir();

        // When running the blocking write.
        let result = render_and_write_blocking(&job("out.pdf", &dir.path));

        // Then it is refused with the exporter wording, and nothing is written.
        let reason = result
            .expect_err("pdf is not a supported export format")
            .to_string();
        assert!(reason.starts_with("no exporter for .pdf"), "was: {reason}");
        assert!(
            !dir.path
                .join("out.pdf")
                .expect("temp root joins a bare name")
                .exists()
        );
    }

    #[rstest::rstest]
    fn the_success_notice_names_the_written_file() {
        // Given a written path inside a session directory.
        let dir = jinn_testutil::tempdir();
        let path = dir
            .path
            .join("dev/project/chat-export.html")
            .expect("temp root joins a bare name");

        // When building the chat notice.
        let notice = written_notice(&path);

        // Then it names the file, not the whole path.
        assert_eq!(notice, "Exported to chat-export.html");
    }

    #[rstest::rstest]
    fn the_success_notice_survives_a_directory_destination() {
        // Given a destination that names a directory.
        let dir = jinn_testutil::tempdir();
        let path = dir
            .path
            .join("dev/project")
            .expect("temp root joins a bare name");

        // When building the chat notice.
        let notice = written_notice(&path);

        // Then it still renders a name rather than an empty one.
        assert_eq!(notice, "Exported to project");
    }

    #[rstest::rstest]
    fn a_text_file_is_refused_with_the_exporter_wording() {
        // Given a job targeting a `.txt` path.
        let dir = jinn_testutil::tempdir();

        // When running the blocking write.
        let result = render_and_write_blocking(&job("chat.txt", &dir.path));

        // Then the user is told there is no exporter for it.
        let reason = result.expect_err("txt has no exporter").to_string();
        assert!(reason.starts_with("no exporter for .txt"), "was: {reason}");
        assert!(
            !dir.path
                .join("chat.txt")
                .expect("temp root joins a bare name")
                .exists()
        );
    }

    #[rstest::rstest]
    fn empty_argument_writes_the_default_filename() {
        // Given a job with no typed path.
        let dir = jinn_testutil::tempdir();

        // When running the blocking write.
        let path = render_and_write_blocking(&job("", &dir.path)).expect("write succeeds");

        // Then the default filename is used.
        assert_eq!(
            path,
            dir.path
                .join("chat-export.html")
                .expect("temp root joins a bare name")
        );
        assert!(path.exists());
    }

    #[rstest::rstest]
    fn unwritable_destination_reports_the_reason() {
        // Given a job targeting a path inside a file, not a directory. The
        // relative argument and the failure needle are both derived from
        // one constructed destination, so the message names the path in
        // whatever spelling this platform resolves it to.
        let dir = jinn_testutil::tempdir();
        let blocker = dir
            .path
            .join("blocker")
            .expect("temp root joins a bare name");
        std::fs::write(&blocker, "i am a file").expect("write");
        let destination = dir
            .path
            .join_all(["blocker", "out.html"])
            .expect("bare-name joins keep the base absolute");
        let argument = destination
            .as_path()
            .strip_prefix(dir.path.as_path())
            .expect("the destination is inside the cwd")
            .to_string_lossy()
            .into_owned();

        // When running the blocking write.
        let result = render_and_write_blocking(&job(&argument, &dir.path));

        // Then the failure names the path that could not be written.
        let reason = result
            .expect_err("writing under a file must fail")
            .to_string();
        let needle = destination.to_string();
        assert!(reason.contains(&needle), "was: {reason}");
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
    async fn actor_over(dir: &MarkedPathBuf<Absolute>) -> (TestHarness, SessionId) {
        let mut app = AppState::default();
        let mut session = jinn_session_state::ChatSessionState::new();
        session.set_cwd(dir.clone());
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
        let dir = jinn_testutil::tempdir();
        let (harness, session_id) = actor_over(&dir.path).await;
        let rec = harness.spawn_recorder::<PushChatEntry>().await;

        // When publishing an export request for that directory.
        let msg = ExportSessionToFile {
            session_id,
            path: dir
                .path
                .join("out.html")
                .expect("temp root joins a bare name"),
        };
        harness.publish(msg).await;

        // Then the chat carries a notice naming the written file. The wait
        // is on the notice itself — the recorder proves the export was
        // processed, and a fixed sleep would only guess at it.
        let pushed =
            jinn_testutil::wait::wait_for_messages(&rec, 1, std::time::Duration::from_secs(5))
                .await
                .into_iter()
                .map(|e| e.entry.text())
                .collect::<Vec<String>>();
        assert_eq!(pushed, vec!["Exported to out.html".to_owned()]);

        // Then the file appears (eventually) on disk.
        let target = dir
            .path
            .join("out.html")
            .expect("temp root joins a bare name");
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
        let dir = jinn_testutil::tempdir();
        let (harness, session_id) = actor_over(&dir.path).await;
        let rec = harness.spawn_recorder::<PushChatEntry>().await;

        // When publishing a request for an extension with no exporter.
        harness
            .publish(ExportSessionToFile {
                session_id,
                path: dir
                    .path
                    .join("chat.txt")
                    .expect("temp root joins a bare name"),
            })
            .await;

        // Then the chat carries an error naming the missing exporter. The
        // wait is on the recorder — a fixed sleep would only guess at when
        // the actor's reply was dispatched, and the read could land
        // before it, leaving the assertion to pass on an empty recorder.
        let pushed =
            jinn_testutil::wait::wait_for_messages(&rec, 1, std::time::Duration::from_secs(5))
                .await
                .into_iter()
                .map(|e| e.entry.text())
                .collect::<Vec<String>>();
        assert_eq!(pushed.len(), 1, "expected one error, got {pushed:?}");
        assert!(
            pushed
                .first()
                .is_some_and(|t| t.contains("no exporter for .txt")),
            "was: {pushed:?}"
        );
        // And no file was created.
        assert!(
            !dir.path
                .join("chat.txt")
                .expect("temp root joins a bare name")
                .exists()
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn an_unknown_session_reports_an_error_in_the_chat() {
        // Given a spawned actor over a temp-dir session.
        let dir = jinn_testutil::tempdir();
        let (harness, _) = actor_over(&dir.path).await;
        let rec = harness.spawn_recorder::<PushChatEntry>().await;

        // When publishing a request naming a session that is not loaded.
        harness
            .publish(ExportSessionToFile {
                session_id: SessionId::new(),
                path: dir
                    .path
                    .join("out.html")
                    .expect("temp root joins a bare name"),
            })
            .await;

        // Then the chat says so rather than staying silent. The wait is on
        // the recorder — a fixed sleep would only guess at when the actor's
        // reply was dispatched, and the read could land before it, leaving
        // the assertion to pass on an empty recorder.
        let pushed =
            jinn_testutil::wait::wait_for_messages(&rec, 1, std::time::Duration::from_secs(5))
                .await
                .into_iter()
                .map(|e| e.entry.text())
                .collect::<Vec<String>>();
        assert_eq!(pushed.len(), 1, "expected one error, got {pushed:?}");
        assert!(
            pushed
                .first()
                .is_some_and(|t| t.contains("no loaded session")),
            "was: {pushed:?}"
        );
    }
}
