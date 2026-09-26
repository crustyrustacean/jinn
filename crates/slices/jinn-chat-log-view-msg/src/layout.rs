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

/// Maximum rendered lines a session preview shows.
///
/// The last entry is what the user is reading, so overflow is dropped from the
/// front — the popup is sized from the surviving lines, which keeps its height
/// honest about what is actually on screen.
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
