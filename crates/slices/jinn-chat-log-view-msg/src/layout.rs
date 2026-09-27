//! Chat-log layout work and results that cross the actor bus.
//!
//! Measuring a large session's chat log means rendering every entry once to
//! learn how many wrapped lines it occupies. On a big session that is far too
//! slow to do on the frame that first shows the history, so the work is handed
//! to a pool of layout workers over the bus: the session load dispatches a
//! [`LayoutChatSession`], a worker measures off the main thread, and it answers
//! with a [`ChatLogLayoutComputed`].
//!
//! The completion actor applies the counts and clears the session load guard —
//! so the loading indication stays up until the chat log has been measured, not
//! merely read from disk. A dead worker or a slow job is caught by the layout
//! supervisor, armed with [`ArmLayoutDeadline`].

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use jinn_core_types::{ChatEntry, ChatEntryId, SessionId};
use jinn_slices::BusMessage;
use serde::{Deserialize, Serialize};

/// Number of trailing history entries a session preview shows.
///
/// A preview is a *glance*, not a transcript: enough tail to recognize where
/// the conversation left off, few enough entries that the wrap work stays
/// bounded no matter how large the session behind it is.
pub const PREVIEW_ENTRY_COUNT: usize = 5;

/// Trailing history entries a preview *request* carries.
///
/// More than [`PREVIEW_ENTRY_COUNT`] because a preview skips entries that are
/// still accumulating tokens, and the requester has to hand the worker enough
/// history to find the same settled window the requester itself found. Carrying
/// only the trailing five would let a reply still in production push the
/// settled entries out of the slice entirely, and the worker would render a
/// shorter window than the one the key was computed over — the two sides would
/// disagree by construction.
///
/// Bounded rather than unbounded: a whole long history copied per keystroke is
/// the cost the trailing slice exists to avoid, and sixteen entries is far more
/// than a five-entry window can ever need.
pub const PREVIEW_REQUEST_ENTRY_COUNT: usize = 16;

/// Rendered columns of an in-production entry's text a continuation marker
/// shows.
///
/// A marker stands in for a reply that may be thousands of lines long. This is
/// enough of its tail to recognize what it is saying and to see it is still
/// going, in a popup whose whole content area is twenty rows.
pub const PREVIEW_MARKER_COLUMNS: usize = 256;

/// Rows a continuation marker may occupy.
///
/// The marker is a status line, not content. It takes a bounded share of the
/// preview's rows and never more than the budget, so a long production reply
/// cannot crowd the settled entries it is standing in for out of the popup.
pub const PREVIEW_MARKER_MAX_ROWS: usize = 8;

/// Whether an entry has stopped producing and can be previewed as settled.
///
/// The one definition of settledness, and it is deliberately a property of the
/// entry alone: the request path computes the preview's key from it, the worker
/// reads the same answer off the entries it was handed, and neither can drift
/// from the other.
///
/// An entry is *not* settled while its `Streamed` timing has no `finished_at`.
/// That covers a `Thinking`, `Assistant`, `Actor`, or `Transient` entry still
/// receiving tokens, and it also covers a `ToolCall` still streaming its
/// arguments: one is created with a `Streamed` timing and only has its
/// `finished_at` set when the call is finalized, so the timing answers for the
/// tool case without a second source of truth.
///
/// `Instant` entries — a user message, a system note, a settled tool result —
/// are settled by construction, which is why this is not merely
/// `finished_at().is_some()`.
#[must_use]
pub fn entry_is_settled(entry: &ChatEntry) -> bool {
    match &entry.timing {
        jinn_core_types::entry_timing::EntryTiming::Instant { .. } => true,
        jinn_core_types::entry_timing::EntryTiming::Streamed { finished_at, .. } => {
            finished_at.is_some()
        }
    }
}

/// Maximum rendered lines a session preview shows.
///
/// The last entry is what the user is reading, so overflow is dropped from the
/// front. This is also the session preview popup's content height, so the box
/// is exactly as tall as the preview it was sized for and the worker cannot
/// hand back more than the popup will show.
pub const PREVIEW_MAX_LINES: usize = 20;

/// Measure one session's chat log off the render thread.
///
/// Carries the history itself rather than a session id, because the chat log's
/// view state only borrows it (`&[ChatEntry]`) and a worker thread cannot hold
/// that borrow. The load actor takes exactly one copy of the entries to escape
/// the borrow, wraps it in an `Arc`, and every subsequent hand-off is a pointer
/// bump — the pool is three workers, and copying per worker would put a
/// multi-megabyte copy on the click that opened the session.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Measure the chat log line counts for a session.")]
pub struct LayoutChatSession {
    /// The session whose chat log should be measured.
    pub session_id: SessionId,
    /// Content width to measure at — the chat log's width minus its gutter.
    pub content_width: u16,
    /// The session's flat history, shared with the worker rather than copied
    /// into it.
    pub entries: Arc<[ChatEntry]>,
    /// Blocks of ignored entries the user has expanded, so the worker
    /// collapses exactly the same entries the renderer will.
    pub shown_ignored_blocks: HashSet<ChatEntryId>,
    /// Minimum contiguous ignored entries required to collapse a block.
    pub min_collapse_count: usize,
    /// Lines before a tool call or result is truncated.
    pub tool_entry_max_lines: u16,
}

/// One entry's measured height, ready to be stored in the line cache.
///
/// Carries the full cache identity, not just the count: the line cache hits
/// only when the entry's content hashes, expanded state, and render variant
/// all match what the measurement saw. A count published without them could
/// never be found again, so the whole measurement would be discarded on the
/// first frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasuredEntryCount {
    /// The entry this count describes.
    pub entry_id: ChatEntryId,
    /// The entry's O(1) content summary, as the worker observed it.
    pub signature: u64,
    /// The entry's full content hash, as the worker observed it.
    pub fingerprint: u64,
    /// Whether the entry was expanded when the count was computed.
    pub is_expanded: bool,
    /// Hash of the status-derived render inputs at compute time.
    pub variant: u64,
    /// The measured wrapped line count.
    pub wrapped_count: u32,
}

/// One session's chat log has been measured.
///
/// Collapsed ignored blocks are absent from [`Self::counts`]: a block is
/// always exactly one line, so there is nothing to measure and nothing to
/// store.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "Chat log layout line counts are available.")]
pub struct ChatLogLayoutComputed {
    /// The session the counts were measured for.
    pub session_id: SessionId,
    /// The content width the counts were measured at.
    pub content_width: u16,
    /// Each measured entry, in visual item order.
    pub counts: Vec<MeasuredEntryCount>,
}

/// Arm the deadline after which a layout job is abandoned.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Arm the chat log layout deadline for a session.")]
pub struct ArmLayoutDeadline {
    /// The session being measured.
    pub session_id: SessionId,
    /// How long to wait before abandoning the job.
    pub after: Duration,
}

/// Announce that the layout deadline passed without a result.
///
/// The supervisor handles this by clearing the session load guard, so an
/// unusually slow measurement degrades to a slow frame rather than a stuck
/// loading indication.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "The chat log layout deadline passed with no result.")]
pub struct LayoutDeadlineExpired {
    /// The session whose layout job was abandoned.
    pub session_id: SessionId,
}

/// A supervised child exhausted its restart budget.
///
/// The shape is fixed by the runtime: the supervision engine builds this
/// envelope itself when a child's restart budget runs out, so the field names
/// must stay as they are. There is no session id on it — a failed worker
/// carries no per-job context — so the supervisor releases whichever session
/// is currently loading.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "A supervised child exhausted its restart budget.")]
pub struct Escalated {
    /// The path of the child that was retired.
    pub escalated: String,
    /// Why the child was retired.
    pub reason: String,
}

impl BusMessage for LayoutChatSession {}
impl BusMessage for ChatLogLayoutComputed {}
impl BusMessage for ArmLayoutDeadline {}
impl BusMessage for LayoutDeadlineExpired {}
impl BusMessage for Escalated {}

/// Render a session's preview lines off the render thread.
///
/// Wrapping the last few entries of a large session is enough work to be felt as
/// a stutter when it happens inside a frame, so the sidebar's preview popup hands
/// it to the same layout pool the chat log measures on. Only the trailing
/// entries travel, so this is small work done in the wrong place — not a repeat
/// of the measurement problem.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Render a session's preview lines off the render thread.")]
pub struct PreviewSessionRequested {
    /// The session to preview.
    pub session_id: SessionId,
    /// The width to wrap the preview at — the popup's inner width.
    pub content_width: u16,
    /// Which request this is.
    ///
    /// Monotonic per sidebar, so a result belonging to a request the cursor has
    /// moved past can be recognised and dropped rather than shown against the
    /// wrong session's content.
    pub generation: u64,
    /// The session's flat history, shared with the worker rather than copied
    /// into it. The worker reads only its tail.
    pub entries: Arc<[ChatEntry]>,
    /// Lines before a tool call or result is truncated.
    pub tool_entry_max_lines: u16,
    /// A summary of the previewed entries' content, as the requester saw it.
    ///
    /// Carried so the result can be checked against current content: a session
    /// that is streaming changes under the request, and a result built from the
    /// older text must not be served as current.
    pub signature: u64,
}

/// A session's preview lines are rendered and available.
///
/// OWNER: `SidebarStateActor` (completes the preview) and the render pass
/// (draws the lines).
///
/// Serde is implemented by hand rather than derived, because the payload is
/// rendered `Line`s and ratatui does not derive serde on its text types —
/// `ratatui-core`'s `serde` feature covers style and layout, not `text`. The
/// manual pair projects through an intermediate type, so this type itself
/// remains the single description of the message. Without it the
/// `Message: Serialize` bound would have to be given up, and the bus's journal
/// door could not encode the event.
#[derive(Debug, Clone, trouper::schema::Event)]
#[schema(description = "Session preview lines are available.")]
pub struct SessionPreviewRendered {
    /// The session the lines describe.
    pub session_id: SessionId,
    /// The request that produced them.
    pub generation: u64,
    /// A summary of the previewed entries' content, as the worker saw it.
    pub signature: u64,
    /// The width the lines were wrapped at.
    pub content_width: u16,
    /// The truncated preview lines.
    ///
    /// Shared rather than copied: the sidebar hands these to the render pass
    /// every frame, and copying them there is the cost this work exists to
    /// remove.
    pub lines: Arc<Vec<ratatui::text::Line<'static>>>,
}

/// The serializable form of [`SessionPreviewRendered`]: its lines flattened to
/// text, since styling is a presentation detail the wire does not need.
#[derive(Serialize, Deserialize)]
struct SessionPreviewRenderedWire {
    session_id: SessionId,
    generation: u64,
    signature: u64,
    content_width: u16,
    lines: Vec<String>,
}

impl Serialize for SessionPreviewRendered {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        SessionPreviewRenderedWire {
            session_id: self.session_id.clone(),
            generation: self.generation,
            signature: self.signature,
            content_width: self.content_width,
            lines: self
                .lines
                .iter()
                .map(|line| {
                    line.spans
                        .iter()
                        .map(|span| span.content.as_ref())
                        .collect::<String>()
                })
                .collect(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SessionPreviewRendered {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = SessionPreviewRenderedWire::deserialize(deserializer)?;
        Ok(Self {
            session_id: wire.session_id,
            generation: wire.generation,
            signature: wire.signature,
            content_width: wire.content_width,
            lines: Arc::new(
                wire.lines
                    .into_iter()
                    .map(ratatui::text::Line::from)
                    .collect(),
            ),
        })
    }
}

/// Arm the deadline after which a preview render is abandoned.
///
/// A preview has no inline fallback the way a chat-log measurement does, so the
/// deadline is what stops a slow or wedged worker from leaving the popup
/// spinning forever. On expiry the preview is dropped and the next cursor move
/// re-requests.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Arm the session preview render deadline.")]
pub struct ArmPreviewDeadline {
    /// The session being previewed.
    pub session_id: SessionId,
    /// Which request the deadline covers, so an expiry cannot clear a newer one.
    pub generation: u64,
    /// How long to wait before abandoning the render.
    pub after: Duration,
}

/// A preview render outlived its deadline and is abandoned.
///
/// Carries the generation so an expiry for a superseded request cannot stop the
/// spinner belonging to the one that replaced it.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A session preview render was abandoned.")]
pub struct PreviewDeadlineExpired {
    /// The session whose preview was abandoned.
    pub session_id: SessionId,
    /// The request that ran out of time.
    pub generation: u64,
}

impl BusMessage for PreviewSessionRequested {}
impl BusMessage for SessionPreviewRendered {}
impl BusMessage for ArmPreviewDeadline {}
impl BusMessage for PreviewDeadlineExpired {}

#[cfg(test)]
mod preview_serde_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::*;

    /// A result carrying `lines`, as a worker would publish.
    fn rendered(lines: &[&str]) -> SessionPreviewRendered {
        SessionPreviewRendered {
            session_id: SessionId::new(),
            generation: 4,
            signature: 99,
            content_width: 40,
            lines: Arc::new(
                lines
                    .iter()
                    .map(|text| ratatui::text::Line::from((*text).to_owned()))
                    .collect(),
            ),
        }
    }

    #[rstest::rstest]
    fn a_preview_result_roundtrips_through_json() {
        // Given a published preview result.
        let original = rendered(&["first", "second"]);

        // When it is encoded and decoded.
        let json = serde_json::to_string(&original).expect("encode");
        let decoded: SessionPreviewRendered = serde_json::from_str(&json).expect("decode");

        // Then every field survived the roundtrip.
        assert_eq!(decoded.session_id, original.session_id);
        assert_eq!(decoded.generation, original.generation);
        assert_eq!(decoded.signature, original.signature);
        assert_eq!(decoded.content_width, original.content_width);
    }

    #[rstest::rstest]
    fn preview_line_text_survives_the_roundtrip() {
        // Given a result with two lines of text.
        let original = rendered(&["first", "second"]);

        // When it is encoded and decoded.
        let json = serde_json::to_string(&original).expect("encode");
        let decoded: SessionPreviewRendered = serde_json::from_str(&json).expect("decode");

        // Then the line text came back, in order.
        let text: Vec<String> = decoded
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect();
        assert_eq!(text, vec!["first", "second"]);
    }
}
