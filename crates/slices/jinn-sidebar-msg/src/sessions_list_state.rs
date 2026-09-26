//! Sessions-list view vocabulary — the entry model, tree prompt state,
//! and preview cache shared between the kernel's session list logic and
//! the sidebar slice.
//!
//! The kernel owns the list logic (building entries from the session
//! map, reconcile on removal); the sidebar slice owns the section's
//! interactions. Both speak these types.

use std::sync::Arc;

use jinn_core_types::SessionId;

pub use jinn_session_list::{SessionEntry, SessionEntryKind};

/// The tree action a confirmation prompt was armed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreePromptAction {
    /// Archive the subtree as-is (`A` key).
    Archive,
    /// Tear down the root, then archive the subtree (`X` key).
    TeardownAndArchive,
}

/// The route-table action string for the archive-subtree key (`A`).
///
/// Shared by the sidebar's route row (which mints the
/// [`crate::DynamicIntent`]) and the kernel's archive-tree-prompt
/// interceptor (which re-keys prompts onto these strings), so a rename
/// breaks compilation instead of silently detaching the confirm press.
pub const TREE_ARCHIVE_ACTION: &str = "archive subtree";

/// The route-table action string for the teardown+archive key (`X`).
///
/// See [`TREE_ARCHIVE_ACTION`] for why this is a shared constant.
pub const TREE_TEARDOWN_ACTION: &str = "teardown+archive tree";

/// State of the archive-tree confirmation prompt.
///
/// OWNER: IntentHandler (armed on the first press of the arming key,
/// consumed when that same key is pressed again, dismissed on any other
/// intent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveTreePrompt {
    /// Armed: the subtree was fully idle at arm time; `count` is the visible
    /// subtree size (selection plus descendants).
    Confirm {
        /// Number of sessions the confirm press will archive.
        count: usize,
        /// Which tree action the confirm press will perform.
        action: TreePromptAction,
    },
    /// Blocked: at least one member is busy; nothing will archive.
    Busy,
}

/// Where the session preview popup is in its load.
///
/// Distinct from the session load guard in the session map, and deliberately so:
/// a preview is not a session switch, so it must not take the guard's single
/// shared slot — doing so would raise the chat log's loading indication for a
/// session that is not switching, and would have two unrelated features fight
/// over one flag.
///
/// OWNER: `SidebarStateActor` (arms and completes) and the render pass (reads
/// the cached lines and records the width it rendered at).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PreviewLoad {
    /// No request in flight, and nothing to show.
    #[default]
    Idle,
    /// A render is in flight for this session at this generation.
    Loading {
        /// The session being previewed.
        session_id: SessionId,
        /// Which request this is.
        ///
        /// Monotonic, because moving the cursor A → B → A leaves two requests
        /// for A in flight, and without a generation the first A's result could
        /// land after the second's and be shown against the wrong content.
        generation: u64,
    },
    /// Rendered lines are available and current.
    Ready {
        /// The session these lines describe.
        session_id: SessionId,
        /// The request that produced them.
        generation: u64,
        /// A summary of the previewed entries' content, so a streaming entry
        /// invalidates the result rather than showing stale text.
        signature: u64,
        /// The width the lines were wrapped at.
        content_width: u16,
        /// The rendered lines.
        ///
        /// Shared, not owned: the render pass reads these every frame, and a
        /// per-frame `Vec` clone of up to 20 styled lines is exactly the cost
        /// this work exists to remove.
        lines: Arc<Vec<ratatui::text::Line<'static>>>,
    },
}

impl PreviewLoad {
    /// Arms a request, discarding anything previously held.
    ///
    /// Bumps the generation so a result from a superseded request is recognisable
    /// when it arrives.
    pub fn request(&mut self, session_id: SessionId) -> u64 {
        let generation = self.generation().saturating_add(1);
        *self = Self::Loading {
            session_id,
            generation,
        };
        generation
    }

    /// Stores a rendered result, returning whether it was current.
    ///
    /// `false` means the result belongs to a request the cursor has already
    /// moved past, and nothing was written.
    ///
    /// Both the session and the generation are checked. The generation alone is
    /// not enough: it counts every request the sidebar has ever made, so a
    /// result for one session can carry the same number as another session's
    /// live request and would otherwise overwrite it.
    pub fn complete(
        &mut self,
        session_id: SessionId,
        generation: u64,
        signature: u64,
        content_width: u16,
        lines: Arc<Vec<ratatui::text::Line<'static>>>,
    ) -> bool {
        if self.generation() != generation || !self.belongs_to(&session_id) {
            return false;
        }
        *self = Self::Ready {
            session_id,
            generation,
            signature,
            content_width,
            lines,
        };
        true
    }

    /// Whether the held request is for `session_id`.
    ///
    /// False when nothing is in flight, so a result arriving with no request
    /// outstanding can never be written.
    fn belongs_to(&self, session_id: &SessionId) -> bool {
        match self {
            Self::Idle => false,
            Self::Loading {
                session_id: held, ..
            }
            | Self::Ready {
                session_id: held, ..
            } => held == session_id,
        }
    }

    /// The generation of the request currently held, or `0` when idle.
    ///
    /// A result whose generation differs from this is stale, which also covers a
    /// result arriving with no request outstanding at all.
    #[must_use]
    pub fn generation(&self) -> u64 {
        match self {
            Self::Idle => 0,
            Self::Loading { generation, .. } | Self::Ready { generation, .. } => *generation,
        }
    }

    /// The cached lines, when they match what the caller wants to draw.
    ///
    /// `None` means the caller must show the loading state — which is what
    /// distinguishes loading from a session that genuinely has nothing to
    /// preview, since that renders as `Ready` with zero lines.
    ///
    /// Borrowed rather than cloned so the render pass can draw straight out of
    /// the shared buffer: a hit costs a refcount, not a copy of the lines.
    #[must_use]
    pub fn cached(
        &self,
        session_id: &SessionId,
        signature: u64,
        content_width: u16,
    ) -> Option<&Arc<Vec<ratatui::text::Line<'static>>>> {
        match self {
            Self::Ready {
                session_id: ready_id,
                signature: ready_signature,
                content_width: ready_width,
                lines,
                ..
            } if ready_id == session_id
                && *ready_signature == signature
                && *ready_width == content_width =>
            {
                Some(lines)
            }
            _ => None,
        }
    }

    /// Drops an in-flight request for `session_id`.
    ///
    /// Id-scoped, like the session map's `clear_load_for`: a preview abandoned
    /// for one session must not strand another session's spinner.
    ///
    /// Drops any held result, returning to `Idle`.
    ///
    /// Used when the rendered lines stop being valid for a reason no request
    /// key can express — a theme change repaints them, and the next request
    /// arrives from the keyboard on the next cursor move.
    pub fn reset(&mut self) {
        *self = Self::Idle;
    }

    /// Generation-scoped as well as session-scoped: a deadline that fires for a
    /// request the cursor has already moved past must not stop the spinner
    /// belonging to the request that replaced it. Returns whether it abandoned
    /// anything.
    pub fn abandon(&mut self, session_id: &SessionId, generation: u64) -> bool {
        let matches_request = matches!(
            self,
            Self::Loading {
                session_id: loading,
                generation: loading_generation,
            } if loading == session_id && *loading_generation == generation
        );
        if matches_request {
            *self = Self::Idle;
        }
        matches_request
    }
}

#[cfg(test)]
mod preview_load_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        // The shared Vec IS the production payload; the helper only names it.
        clippy::rc_buffer,
        reason = "test code"
    )]

    use super::*;

    /// The lines a worker would have produced for a preview.
    fn lines(text: &str) -> Arc<Vec<ratatui::text::Line<'static>>> {
        Arc::new(vec![ratatui::text::Line::from(text.to_owned())])
    }

    #[rstest::rstest]
    fn complete_discards_a_result_from_an_older_generation() {
        // Given a request in flight, superseded by a second one.
        let mut load = PreviewLoad::default();
        load.request(SessionId::new());
        let stale = load.request(SessionId::new());

        // When the first request's result arrives late.
        let accepted = load.complete(
            SessionId::new(),
            stale.saturating_sub(1),
            7,
            40,
            lines("stale"),
        );

        // Then it is refused and the current request is untouched.
        assert!(!accepted, "a superseded result must not be stored");
        assert_eq!(
            load,
            PreviewLoad::Loading {
                session_id: match &load {
                    PreviewLoad::Loading { session_id, .. } => session_id.clone(),
                    _ => unreachable!("just armed"),
                },
                generation: stale,
            },
            "the in-flight request must survive a late result"
        );
    }

    #[rstest::rstest]
    fn complete_stores_a_result_for_the_current_generation() {
        // Given a request in flight.
        let session_id = SessionId::new();
        let mut load = PreviewLoad::default();
        let generation = load.request(session_id.clone());

        // When its result arrives.
        let accepted = load.complete(session_id.clone(), generation, 7, 40, lines("hello"));

        // Then it becomes the renderable state.
        assert!(accepted, "a current result must be stored");
        assert!(matches!(load, PreviewLoad::Ready { .. }));
    }

    #[rstest::rstest]
    fn cached_misses_when_the_content_width_differs() {
        // Given a preview rendered at one width.
        let session_id = SessionId::new();
        let mut load = PreviewLoad::default();
        let generation = load.request(session_id.clone());
        load.complete(session_id.clone(), generation, 7, 40, lines("hello"));

        // When asked for a different width.
        let hit = load.cached(&session_id, 7, 60);

        // Then there is nothing to draw — the lines would be wrapped wrong.
        assert!(hit.is_none(), "a resize must invalidate the cached lines");
    }

    #[rstest::rstest]
    fn cached_misses_when_the_previewed_content_changed() {
        // Given a preview of a session whose entry is streaming.
        let session_id = SessionId::new();
        let mut load = PreviewLoad::default();
        let generation = load.request(session_id.clone());
        load.complete(session_id.clone(), generation, 7, 40, lines("partial answ"));

        // When the entry's content has since changed.
        let hit = load.cached(&session_id, 8, 40);

        // Then there is nothing to draw — serving it would show stale text.
        assert!(
            hit.is_none(),
            "a streaming entry must invalidate the preview, not serve stale text"
        );
    }

    #[rstest::rstest]
    fn cached_hits_for_matching_session_signature_and_width() {
        // Given a completed preview.
        let session_id = SessionId::new();
        let mut load = PreviewLoad::default();
        let generation = load.request(session_id.clone());
        load.complete(session_id.clone(), generation, 7, 40, lines("hello"));

        // When asked for exactly that.
        let hit = load.cached(&session_id, 7, 40);

        // Then the lines come back.
        let hit = hit.expect("matching lookup should hit");
        assert_eq!(hit.len(), 1, "one line was rendered");
    }

    #[rstest::rstest]
    fn abandon_clears_only_the_named_session() {
        // Given a request in flight for one session.
        let in_flight = SessionId::new();
        let other = SessionId::new();
        let mut load = PreviewLoad::default();
        let generation = load.request(in_flight.clone());

        // When another session's request is abandoned.
        load.abandon(&other, generation);

        // Then the in-flight request is untouched.
        assert!(
            matches!(load, PreviewLoad::Loading { .. }),
            "abandoning one session must not strand another's spinner"
        );
    }

    #[rstest::rstest]
    fn abandon_clears_the_named_sessions_request() {
        // Given a request in flight.
        let in_flight = SessionId::new();
        let mut load = PreviewLoad::default();
        let generation = load.request(in_flight.clone());

        // When that request is abandoned.
        load.abandon(&in_flight, generation);

        // Then nothing is in flight.
        assert_eq!(load, PreviewLoad::Idle, "a stuck spinner must be clearable");
    }

    #[rstest::rstest]
    fn request_bumps_the_generation() {
        // Given a state that has already served one request.
        let mut load = PreviewLoad::default();
        let first = load.request(SessionId::new());

        // When a second request is armed.
        let second = load.request(SessionId::new());

        // Then the generation moved on, so the first result is recognisable.
        assert_eq!(
            second,
            first + 1,
            "each request must advance the generation"
        );
    }

    #[rstest::rstest]
    fn request_discards_a_completed_preview() {
        // Given a ready preview.
        let mut load = PreviewLoad::default();
        let generation = load.request(SessionId::new());
        load.complete(SessionId::new(), generation, 7, 40, lines("hello"));

        // When a new request is armed.
        load.request(SessionId::new());

        // Then the old lines are gone — the render pass must show a spinner
        // rather than the previous session's text.
        assert!(
            matches!(load, PreviewLoad::Loading { .. }),
            "a new request must not leave stale lines on screen"
        );
    }
}

/// The preview's keying, which is what makes a streaming session preview live.
///
/// The key is the previewed entries' *content*, not the history's length. A
/// streaming assistant appends to an entry many times before the history grows
/// at all, so a length key serves stale text for the whole turn.
#[cfg(test)]
mod preview_freshness_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use ratatui::text::Line;

    /// A preview carrying `text`, ready for `session_id` at `content_width`.
    fn ready(
        load: &mut PreviewLoad,
        session_id: &SessionId,
        signature: u64,
        content_width: u16,
        text: &'static str,
    ) {
        let generation = load.request(session_id.clone());
        load.complete(
            session_id.clone(),
            generation,
            signature,
            content_width,
            Arc::new(vec![Line::from(text)]),
        );
    }

    #[rstest::rstest]
    fn unchanged_content_is_served_from_the_same_generation() {
        // Given a preview rendered for a session at a width.
        let id = SessionId::new();
        let mut load = PreviewLoad::default();
        ready(&mut load, &id, 7, 40, "hello");

        // When the render pass asks for the same session, width, and content.
        let cached = load.cached(&id, 7, 40);

        // Then it is served without a new request.
        assert!(cached.is_some(), "unchanged content must be a cache hit");
    }

    #[rstest::rstest]
    fn a_streamed_token_invalidates_the_preview() {
        // Given a preview rendered before a token arrived.
        let id = SessionId::new();
        let mut load = PreviewLoad::default();
        ready(&mut load, &id, 7, 40, "hel");

        // When the render pass asks again with the entry's new content.
        let cached = load.cached(&id, 8, 40);

        // Then it is a miss, so the popup shows its spinner rather than the
        // text the assistant had already replaced.
        assert!(
            cached.is_none(),
            "a streamed token must invalidate the preview"
        );
    }

    #[rstest::rstest]
    fn a_resize_invalidates_the_preview() {
        // Given a preview rendered at one width.
        let id = SessionId::new();
        let mut load = PreviewLoad::default();
        ready(&mut load, &id, 7, 40, "hello");

        // When the terminal is resized.
        let cached = load.cached(&id, 7, 80);

        // Then it is a miss, so the lines are re-wrapped for the new width.
        assert!(cached.is_none(), "a resize must invalidate the preview");
    }

    #[rstest::rstest]
    fn another_session_does_not_invalidate_the_preview() {
        // Given a preview rendered for one session.
        let id = SessionId::new();
        let mut load = PreviewLoad::default();
        ready(&mut load, &id, 7, 40, "hello");

        // When the cursor moves to a different session with the same signature.
        let cached = load.cached(&SessionId::new(), 7, 40);

        // Then it is a miss: the lines belong to the session left behind.
        assert!(
            cached.is_none(),
            "another session's preview must not be served"
        );
    }

    #[rstest::rstest]
    fn a_served_preview_is_shared_rather_than_copied() {
        // Given a preview holding rendered lines.
        let id = SessionId::new();
        let mut load = PreviewLoad::default();
        ready(&mut load, &id, 7, 40, "hello");

        // When it is read twice for the same content.
        let first = load.cached(&id, 7, 40).expect("hit");
        let second = load.cached(&id, 7, 40).expect("hit");

        // Then both reads are the same allocation, not two copies.
        assert!(
            std::sync::Arc::ptr_eq(first, second),
            "a cache hit must not copy the lines"
        );
    }

    #[rstest::rstest]
    fn an_idle_preview_serves_nothing() {
        // Given a preview that was never requested.
        let load = PreviewLoad::default();

        // When the render pass asks for lines anyway.
        let cached = load.cached(&SessionId::new(), 7, 40);

        // Then there is nothing, which is what shows the spinner.
        assert!(cached.is_none());
    }
}
