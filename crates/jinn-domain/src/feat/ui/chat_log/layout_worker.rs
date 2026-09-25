//! [`LayoutWorkerActor`] — measures a loaded session's chat log off the render thread.
//!
//! Loading a large session from disk is not what makes the UI freeze: the freeze
//! is the first frame that follows, which has to know how many wrapped lines every
//! entry occupies before it can work out the scroll extent. That measurement is
//! pure computation over the session's history, so it belongs off the main
//! thread.
//!
//! A pool of these workers shares the work. They are reached with a typed
//! `send_to_any`, which round-robins across every worker that declared
//! [`LayoutChatSession`], and the history travels as a live value rather than
//! serialized — the whole point of the message carrying the entries instead of a
//! session id.
//!
//! The measurement is deliberately the *same* arithmetic the render pass
//! performs, in [`super::history`], so the counts it publishes are the counts the
//! renderer would have computed. That is why this actor lives beside the
//! renderer rather than in the slice: the pieces that must agree — the tool
//! result pairing, the streaming and subagent-waiting flags, the wrap counting —
//! are private to the renderer, and duplicating them would guarantee drift.

use std::collections::HashMap;

use error_stack::Report;
use jinn_chat_log_view::chat_log::{
    ContentIdentity, MeasuredLineCount, RenderContext, entry_to_lines,
};
use jinn_chat_log_view_msg::{
    ChatLogLayoutComputed, LayoutChatSession, MeasuredEntryCount, PROXIMITY_COUNT, VisualItem,
};
use ratatui::widgets::{Paragraph, Wrap};
use trouper::actor::{ActorPath, MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;

use crate::common::state::State;
use crate::feat::ui::chat_log::history::LayoutInputs;

/// Static path the layout worker pool spawns at (one pool per process).
pub const LAYOUT_WORKER_POOL_SIZE: usize = 3;

/// The pool's path prefix; worker `n` spawns at `jinn.chat_log.layout.worker.{n}`.
pub const LAYOUT_WORKER_PATH_PREFIX: &str = "jinn.chat_log.layout.worker.";

/// The path the layout worker at `index` spawns at.
#[must_use]
pub fn layout_worker_path(index: usize) -> ActorPath {
    ActorPath::new(format!("{LAYOUT_WORKER_PATH_PREFIX}{index}"))
}

/// Dependencies for [`LayoutWorkerActor`].
#[derive(Clone)]
pub struct LayoutWorkerActorDeps {
    /// Shared application state, for the per-entry render inputs.
    pub state: State,
}

/// Measures a session's chat log and publishes the resulting line counts.
pub struct LayoutWorkerActor {
    /// Shared application state.
    state: State,
}

impl ServiceActor for LayoutWorkerActor {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "ServiceActor::start is async by trait contract"
    )]
    async fn start(_args: &trouper::json::Json) -> Result<Self, Report<RegistryError>> {
        // Never called: spawned via `spawn`'s start_with (typed deps can't
        // ride the JSON args).
        Err(Report::new(RegistryError::InvalidSpec)
            .attach("LayoutWorkerActor spawns via start_with"))
    }
}

impl LayoutWorkerActor {
    /// Spawns one worker of the layout pool at `index`.
    ///
    /// Every worker declares the same work message, which is what lets
    /// `send_to_any` distribute a job across the pool.
    ///
    /// # Panics
    ///
    /// Panics if the actor's path is already taken — a wiring bug.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "port convention: spawn takes owned deps and clones into start_with"
    )]
    pub fn spawn(
        system: &trouper::system::ActorSystem,
        index: usize,
        deps: LayoutWorkerActorDeps,
    ) -> ActorPath {
        let path = layout_worker_path(index);
        trouper::builder::spawn_service_builder::<Self>(system)
            .at(path.clone())
            .start_with({
                let deps = deps.clone();
                move || {
                    let deps = deps.clone();
                    Box::pin(async move { Ok(Self { state: deps.state }) })
                }
            })
            .handles::<LayoutChatSession>()
            // The result leaves through ctx.publish; the flush gate drops any
            // outbound type not declared here.
            .emits::<ChatLogLayoutComputed>()
            .mailbox(64, trouper::inbox::OverloadPolicy::Block)
            .start();
        path
    }
}

impl MsgHandler<LayoutChatSession> for LayoutWorkerActor {
    async fn handle(&mut self, msg: &LayoutChatSession, ctx: &mut MsgCtx<'_>) {
        // The per-entry render inputs (expanded set, streaming tool calls,
        // subagent phases) live in application state, so they are snapshotted
        // once here rather than threaded through the message.
        let inputs = LayoutInputs::snapshot(&self.state.read(), &msg.session_id);

        // Measuring a large history is seconds of pure CPU. Off the actor's
        // thread so a long job cannot stall the pool's other messages.
        //
        // The handler only lends the message, and a `spawn_blocking` closure
        // must own everything it touches, so the job is cloned. That is one
        // deep copy of the history per measurement — small next to the
        // rendering the measurement itself performs, and far cheaper than
        // doing that rendering on the frame that first shows the history.
        let job = msg.clone();
        let measured = tokio::task::spawn_blocking(move || measure(&job, &inputs))
            .await
            .unwrap_or_default();

        ctx.publish(ChatLogLayoutComputed {
            session_id: msg.session_id.clone(),
            content_width: msg.content_width,
            counts: measured
                .into_iter()
                .map(|count| MeasuredEntryCount {
                    entry_id: count.id,
                    signature: count.content.signature,
                    fingerprint: count.content.fingerprint,
                    is_expanded: count.is_expanded,
                    variant: count.variant,
                    wrapped_count: count.wrapped_count,
                })
                .collect(),
        });
    }
}

/// Measures every entry's wrapped line count for one session.
///
/// Reproduces the render pass's arithmetic exactly: the same visual item
/// projection, the same tool result pairing, the same wrap counting. The
/// rendered lines are discarded as soon as they are counted, so a measurement
/// costs no lasting memory beyond the counts themselves.
pub(crate) fn measure(msg: &LayoutChatSession, inputs: &LayoutInputs) -> Vec<MeasuredLineCount> {
    let visual_items = jinn_chat_log_view_msg::build_visual_items(
        &msg.entries,
        &msg.shown_ignored_blocks,
        PROXIMITY_COUNT,
        msg.min_collapse_count,
    );
    let tool_result_statuses = pair_tool_results(&msg.entries);

    let mut measured = Vec::with_capacity(visual_items.len());
    for item in &visual_items {
        let VisualItem::Entry(history_index) = item else {
            // A collapsed block is always exactly one line, so there is
            // nothing to measure and nothing to store.
            continue;
        };
        let Some(entry) = msg.entries.get(*history_index) else {
            continue;
        };
        measured.push(measure_entry(entry, &tool_result_statuses, inputs, msg));
    }
    measured
}

/// Measures a single entry's wrapped line count.
fn measure_entry(
    entry: &jinn_core_types::ChatEntry,
    tool_result_statuses: &HashMap<String, jinn_core_types::ToolResultStatus>,
    inputs: &LayoutInputs,
    msg: &LayoutChatSession,
) -> MeasuredLineCount {
    use jinn_core_types::ChatEntryKind;

    let is_expanded = inputs.is_expanded(&entry.id);
    let paired_status = match &entry.kind {
        ChatEntryKind::ToolCall { id, .. } => tool_result_statuses.get(id).copied(),
        ChatEntryKind::ToolResult { status, .. } => Some(*status),
        _ => None,
    };
    let is_streaming = inputs.is_streaming(&entry.id);
    let is_waiting_on_subagent = inputs.is_task_waiting(entry, tool_result_statuses);

    let ctx = RenderContext {
        content_width: msg.content_width,
        is_selected: false,
        is_expanded,
        tool_entry_max_lines: msg.tool_entry_max_lines,
        theme: inputs.theme().clone(),
        paired_status,
        is_streaming,
        is_waiting_on_subagent,
    };
    let lines = entry_to_lines(entry, &ctx);
    let wrapped_count = wrapped_line_count(&lines, msg.content_width);

    MeasuredLineCount {
        id: entry.id.clone(),
        content: ContentIdentity {
            signature: entry.content_signature(),
            fingerprint: entry.content_fingerprint(),
        },
        is_expanded,
        variant: crate::feat::ui::chat_log::history::render_variant(
            paired_status,
            is_streaming,
            is_waiting_on_subagent,
        ),
        wrapped_count,
    }
}

/// How many wrapped lines `lines` occupies at `content_width`.
///
/// A width of zero means "do not wrap", matching the render pass.
fn wrapped_line_count(lines: &[ratatui::text::Line<'static>], content_width: u16) -> u32 {
    if content_width == 0 {
        return u32::try_from(lines.len()).unwrap_or(u32::MAX);
    }
    Paragraph::new(lines.to_vec())
        .wrap(Wrap { trim: false })
        .line_count(content_width) as u32
}

/// Pairs each tool result with its call so the renderer can tint the call's
/// background by the result's status.
fn pair_tool_results(
    entries: &[jinn_core_types::ChatEntry],
) -> HashMap<String, jinn_core_types::ToolResultStatus> {
    entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            jinn_core_types::ChatEntryKind::ToolResult { id, status, .. } => {
                Some((id.clone(), *status))
            }
            _ => None,
        })
        .collect()
}
