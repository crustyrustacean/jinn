//! Sessions-list view vocabulary — the entry model, tree prompt state,
//! and preview cache shared between the kernel's session list logic and
//! the sidebar slice.
//!
//! The kernel owns the list logic (building entries from the session
//! map, reconcile on removal); the sidebar slice owns the section's
//! interactions. Both speak these types.

use std::collections::HashMap;
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

/// How many sessions' previews are held before the least-recently-used is
/// dropped.
///
/// A preview is at most `PREVIEW_MAX_LINES` (20) styled lines, so a full cache
/// is a few hundred KB — small enough to hold without a size budget, bounded
/// enough that a project with hundreds of sessions does not accumulate one per
/// session for the life of the process.
pub const PREVIEW_CACHE_CAPACITY: usize = 32;

/// One session's rendered preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedPreview {
    /// A summary of the previewed entries' content, so a streaming entry
    /// invalidates the result rather than showing stale text.
    pub signature: u64,
    /// The width the lines were wrapped at.
    pub content_width: u16,
    /// The rendered lines.
    ///
    /// Shared, not owned: the render pass reads these every frame, and a
    /// per-frame `Vec` clone of up to 20 styled lines is exactly the cost this
    /// work exists to remove.
    pub lines: Arc<Vec<ratatui::text::Line<'static>>>,
}

/// A render running for one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InFlightPreview {
    /// Which request this is. Monotonic per sidebar.
    pub generation: u64,
    /// The content the request was made against.
    pub signature: u64,
    /// The width the request was made at.
    pub content_width: u16,
}

/// Every preview the sidebar holds: finished renders, keyed by session, plus
/// what is currently running.
///
/// Distinct from the session load guard in the session map, and deliberately so:
/// a preview is not a session switch, so it must not take the guard's single
/// shared slot — doing so would raise the chat log's loading indication for a
/// session that is not switching, and would have two unrelated features fight
/// over one flag.
///
/// The two maps are separate because the two questions are separate. The cache
/// answers "what can I draw", and is keyed by session so leaving a session does
/// not destroy what was rendered for it. The in-flight map answers "is this
/// render worth requesting again", and is keyed by session so supersession is
/// per-session rather than a single global counter that one session's result
/// could collide with another's.
///
/// OWNER: `SidebarStateActor` (arms, completes, and abandons) and the render
/// pass (reads the cached lines and records the width it rendered at).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviewLoad {
    /// Finished previews by session.
    cache: HashMap<SessionId, CachedPreview>,
    /// Least-recently-used first, parallel to `cache`'s keys.
    order: Vec<SessionId>,
    /// Renders running, by the session they were made for.
    in_flight: HashMap<SessionId, InFlightPreview>,
    /// The generation below which a result was computed against a theme that has
    /// since been replaced, and must not be written.
    reset_floor: u64,
    /// The next generation to hand out. Never decremented.
    next_generation: u64,
}

impl PreviewLoad {
    /// The cached lines, when they match what the caller wants to draw.
    ///
    /// `None` means the caller must show the loading state — which is what
    /// distinguishes loading from a session that genuinely has nothing to
    /// preview, since that renders as a cache hit holding zero lines.
    ///
    /// Takes `&self` and takes no recency: the render pass calls this under the
    /// sections' read lock, every frame, and an LRU touch would push it onto a
    /// write lock for a heuristic. Recency is refreshed by [`Self::touch`] on
    /// the keyboard path, which already knows which session the cursor is on.
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
        self.cache
            .get(session_id)
            .filter(|entry| entry.signature == signature && entry.content_width == content_width)
            .map(|entry| &entry.lines)
    }

    /// Marks a session as just-used, so it is not the next one evicted.
    ///
    /// Called from the keyboard path rather than from [`Self::cached`]: the
    /// render pass reads under a read lock and cannot touch, and recency is
    /// about which sessions the user is actually visiting.
    pub fn touch(&mut self, session_id: &SessionId) {
        let Some(index) = self.order.iter().position(|id| id == session_id) else {
            return;
        };
        let id = self.order.remove(index);
        self.order.push(id);
    }

    /// Arms a request for `session_id`, returning its generation.
    ///
    /// Bumps the generation so a result from a superseded request for the *same*
    /// session is recognisable. Other sessions' in-flight entries and the whole
    /// cache are untouched — a preview is not a session switch, so requesting one
    /// must not cost the user every other preview they have already paid for.
    pub fn request(&mut self, session_id: SessionId, signature: u64, content_width: u16) -> u64 {
        let generation = self.next_generation;
        self.next_generation = self.next_generation.saturating_add(1);
        self.in_flight.insert(
            session_id,
            InFlightPreview {
                generation,
                signature,
                content_width,
            },
        );
        generation
    }

    /// Whether a render is running for this session at all, whatever its content.
    ///
    /// Distinct from [`Self::in_flight_matches`], which answers "is this exact
    /// request running". This one answers "is anything running", which is the
    /// only way to tell a stuck preview from one that is merely busy — and to
    /// observe the deadline actually firing, since the render it is waiting on
    /// never comes back in the test that checks for it.
    #[must_use]
    pub fn is_in_flight_for(&self, session_id: &SessionId) -> bool {
        self.in_flight.contains_key(session_id)
    }

    /// Whether an identical request is already running.
    ///
    /// The render pass cannot publish, so a duplicate request has to be stopped
    /// here: rapid navigation otherwise queues one render per keystroke on a
    /// pool that is already busy with chat-log measurement.
    #[must_use]
    pub fn in_flight_matches(
        &self,
        session_id: &SessionId,
        signature: u64,
        content_width: u16,
    ) -> bool {
        self.in_flight.get(session_id).is_some_and(|entry| {
            entry.signature == signature && entry.content_width == content_width
        })
    }

    /// Stores a rendered result, returning whether it was kept.
    ///
    /// Three cases, in order:
    ///
    /// - It is the live request for this session. Kept.
    /// - A *newer* request for this session is running, so the user has already
    ///   moved past this content. Refused: it was computed against text the user
    ///   has replaced, and a stale write would overwrite the newer request's
    ///   eventual result.
    /// - No request is outstanding for this session — a result whose deadline
    ///   already fired, or whose request was reset. Kept anyway: the work was
    ///   paid for, and refusing it is what strands the popup on a spinner. It is
    ///   still safe to cache because [`Self::cached`] re-checks the signature
    ///   and width per session, so whether it is ever *served* is decided at
    ///   lookup, not here.
    pub fn complete(
        &mut self,
        session_id: SessionId,
        generation: u64,
        signature: u64,
        content_width: u16,
        lines: Arc<Vec<ratatui::text::Line<'static>>>,
    ) -> bool {
        if !self.accepts(&session_id, generation) {
            // Rare and worth a line: a rejection means rendered work was thrown
            // away, and the generation fields say which rule refused it.
            tracing::warn!(
                session_id = %session_id, generation, signature, content_width,
                in_flight_generation = self.in_flight.get(&session_id).map(|e| e.generation),
                reset_floor = self.reset_floor,
                next_generation = self.next_generation,
                "preview result rejected",
            );
            return false;
        }
        self.in_flight.remove(&session_id);
        self.insert_cached(
            session_id,
            CachedPreview {
                signature,
                content_width,
                lines,
            },
        );
        true
    }

    /// Whether a result for `session_id` at `generation` may be written.
    ///
    /// The generation floor rejects results built against a theme that has since
    /// been replaced. Without it, a render in flight during a theme change would
    /// land in the cache with the old theme's colors — a regression the single
    /// slot avoided only by accident, because going `Idle` happened to reject
    /// everything.
    fn accepts(&self, session_id: &SessionId, generation: u64) -> bool {
        if generation < self.reset_floor {
            return false;
        }
        match self.in_flight.get(session_id) {
            // A live request: only its own result may land.
            Some(entry) => entry.generation == generation,
            // A newer request for this session supersedes it.
            None if generation < self.next_generation => true,
            None => false,
        }
    }

    /// Drops the in-flight entry for `session_id` at `generation`.
    ///
    /// Id-scoped, like the session map's `clear_load_for`: a preview abandoned
    /// for one session must not strand another session's spinner.
    ///
    /// Generation-scoped as well: a deadline that fires for a request the cursor
    /// has already moved past must not stop the spinner belonging to the request
    /// that replaced it.
    ///
    /// The cache is deliberately left alone. A render that lands after its
    /// deadline was still paid for, and dropping it is exactly what made the
    /// popup spin forever. Returns whether it abandoned anything.
    pub fn abandon(&mut self, session_id: &SessionId, generation: u64) -> bool {
        let matches_request = self
            .in_flight
            .get(session_id)
            .is_some_and(|entry| entry.generation == generation);
        if matches_request {
            self.in_flight.remove(session_id);
        }
        matches_request
    }

    /// Drops every cached preview and every in-flight request.
    ///
    /// Used when the rendered lines stop being valid for a reason no request key
    /// can express — a theme change repaints them, and the next cursor move
    /// re-requests. The generation floor it leaves behind is what stops a render
    /// that was already running from writing old-theme lines back into the
    /// cache it just cleared.
    pub fn reset(&mut self) {
        // Rare (a theme change) and worth a line: it drops every cached preview,
        // so a spinner appearing for many sessions at once has a cause here.
        tracing::warn!(
            reset_floor = self.reset_floor,
            next_generation = self.next_generation,
            cached = self.cache.len(),
            in_flight = self.in_flight.len(),
            "preview cache reset; every preview will re-render",
        );
        self.cache.clear();
        self.order.clear();
        self.in_flight.clear();
        self.reset_floor = self.next_generation;
    }

    /// Inserts, dropping the least-recently-used entries past the cap.
    fn insert_cached(&mut self, session_id: SessionId, entry: CachedPreview) {
        if let Some(index) = self.order.iter().position(|id| *id == session_id) {
            self.order.remove(index);
        }
        self.cache.insert(session_id.clone(), entry);
        self.order.push(session_id);
        while self.order.len() > PREVIEW_CACHE_CAPACITY {
            // `remove(0)` memmoves at most `PREVIEW_CACHE_CAPACITY` pointers, and
            // only past the cap — a bounded cost, not an accidental quadratic.
            let eldest = self.order.remove(0);
            self.cache.remove(&eldest);
        }
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
    pub(super) fn lines(text: &str) -> Arc<Vec<ratatui::text::Line<'static>>> {
        Arc::new(vec![ratatui::text::Line::from(text.to_owned())])
    }

    /// Arms and completes a request in one step, leaving `session_id` cached.
    fn serve(load: &mut PreviewLoad, session_id: &SessionId) {
        let generation = load.request(session_id.clone(), 7, 40);
        load.complete(session_id.clone(), generation, 7, 40, lines("hello"));
    }

    #[rstest::rstest]
    fn complete_discards_a_result_from_an_older_generation() {
        // Given a request in flight, superseded by a second one.
        let session_id = SessionId::new();
        let mut load = PreviewLoad::default();
        load.request(session_id.clone(), 7, 40);
        let stale = load.request(session_id.clone(), 8, 40);

        // When the first request's result arrives late.
        let accepted = load.complete(
            session_id.clone(),
            stale.saturating_sub(1),
            7,
            40,
            lines("stale"),
        );

        // Then it is refused, because a newer request for the same session owns
        // the slot and this result was built against content it replaced.
        assert!(!accepted, "a superseded result must not be stored");
    }

    #[rstest::rstest]
    fn complete_stores_a_result_for_the_current_generation() {
        // Given a request in flight.
        let session_id = SessionId::new();
        let mut load = PreviewLoad::default();
        let generation = load.request(session_id.clone(), 7, 40);

        // When its result arrives.
        let accepted = load.complete(session_id.clone(), generation, 7, 40, lines("hello"));

        // Then it becomes the renderable state.
        assert!(accepted, "a current result must be stored");
        assert!(load.cached(&session_id, 7, 40).is_some());
    }

    #[rstest::rstest]
    fn cached_misses_when_the_content_width_differs() {
        // Given a preview rendered at one width.
        let session_id = SessionId::new();
        let mut load = PreviewLoad::default();
        serve(&mut load, &session_id);

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
        serve(&mut load, &session_id);

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
        serve(&mut load, &session_id);

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
        let generation = load.request(in_flight.clone(), 7, 40);

        // When another session's request is abandoned.
        load.abandon(&other, generation);

        // Then the in-flight request is untouched.
        assert!(
            load.in_flight_matches(&in_flight, 7, 40),
            "abandoning one session must not strand another's spinner"
        );
    }

    #[rstest::rstest]
    fn abandon_clears_the_named_sessions_request() {
        // Given a request in flight.
        let in_flight = SessionId::new();
        let mut load = PreviewLoad::default();
        let generation = load.request(in_flight.clone(), 7, 40);

        // When that request is abandoned.
        load.abandon(&in_flight, generation);

        // Then nothing is in flight, so a fresh request is not suppressed.
        assert!(
            !load.in_flight_matches(&in_flight, 7, 40),
            "a stuck spinner must be clearable"
        );
    }

    #[rstest::rstest]
    fn request_bumps_the_generation() {
        // Given a state that has already served one request.
        let mut load = PreviewLoad::default();
        let first = load.request(SessionId::new(), 7, 40);

        // When a second request is armed.
        let second = load.request(SessionId::new(), 7, 40);

        // Then the generation moved on, so the first result is recognisable.
        assert_eq!(
            second,
            first + 1,
            "each request must advance the generation"
        );
    }

    #[rstest::rstest]
    fn request_leaves_another_sessions_preview_cached() {
        // Given a session whose preview is already rendered.
        let first = SessionId::new();
        let mut load = PreviewLoad::default();
        serve(&mut load, &first);

        // When a request is armed for a different session.
        let second = SessionId::new();
        load.request(second.clone(), 7, 40);

        // Then the first session's lines are still there — a preview is not a
        // session switch, so asking for one must not destroy the others.
        assert!(
            load.cached(&first, 7, 40).is_some(),
            "a new request must not evict an unrelated session's preview"
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
    pub(super) fn ready(
        load: &mut PreviewLoad,
        session_id: &SessionId,
        signature: u64,
        content_width: u16,
        text: &'static str,
    ) {
        let generation = load.request(session_id.clone(), signature, content_width);
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
    fn a_cached_preview_survives_leaving_its_session() {
        // Given two sessions, both with a rendered preview.
        let first = SessionId::new();
        let second = SessionId::new();
        let mut load = PreviewLoad::default();
        ready(&mut load, &first, 7, 40, "first");
        ready(&mut load, &second, 7, 40, "second");

        // When the cursor comes back to the first.
        let cached = load.cached(&first, 7, 40);

        // Then it is a hit, served from memory rather than re-rendered.
        assert!(
            cached.is_some(),
            "leaving a session must not destroy the preview paid for it"
        );
    }

    #[rstest::rstest]
    fn a_session_serves_its_own_preview_after_another_lands() {
        // Given one session's preview cached and another's still in flight.
        let first = SessionId::new();
        let second = SessionId::new();
        let mut load = PreviewLoad::default();
        ready(&mut load, &first, 7, 40, "first");
        load.request(second.clone(), 7, 40);

        // When the cursor returns to the first session.
        let cached = load.cached(&first, 7, 40);

        // Then it is a hit, unaffected by the other session's pending render.
        assert!(
            cached.is_some(),
            "an unrelated in-flight render must not hide a cached preview"
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

    #[rstest::rstest]
    fn an_empty_preview_is_served_as_empty() {
        // Given a session with no entries, whose render returned zero lines.
        let id = SessionId::new();
        let mut load = PreviewLoad::default();
        let generation = load.request(id.clone(), 7, 40);
        load.complete(id.clone(), generation, 7, 40, Arc::new(Vec::new()));

        // When the render pass asks for its lines.
        let cached = load.cached(&id, 7, 40);

        // Then it is a hit holding nothing. A miss here would spin forever on a
        // session that has genuinely completed, because re-requesting an empty
        // history returns empty again.
        assert!(
            cached.is_some(),
            "an empty session is complete, not loading"
        );
    }
}

/// A result arriving after the deadline that armed it.
///
/// The deadline drops the in-flight request, but the render it was watching for
/// is still running on a shared pool and still finishes. Refusing that work is
/// what left the popup spinning with nothing behind it.
#[cfg(test)]
mod preview_late_result_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::preview_load_tests::lines;
    use super::*;

    #[rstest::rstest]
    fn a_result_after_its_deadline_is_cached() {
        // Given a request whose deadline has already fired.
        let session_id = SessionId::new();
        let mut load = PreviewLoad::default();
        let generation = load.request(session_id.clone(), 7, 40);
        load.abandon(&session_id, generation);

        // When its result finally lands.
        let accepted = load.complete(session_id.clone(), generation, 7, 40, lines("late"));

        // Then it is kept, so the popup recovers without another cursor move.
        assert!(
            accepted,
            "a late result was paid for and must not be discarded"
        );
        assert!(load.cached(&session_id, 7, 40).is_some());
    }

    #[rstest::rstest]
    fn a_late_result_for_one_session_leaves_another_in_flight() {
        // Given two requests in flight, one of which is abandoned.
        let abandoned = SessionId::new();
        let live = SessionId::new();
        let mut load = PreviewLoad::default();
        let stale = load.request(abandoned.clone(), 7, 40);
        load.request(live.clone(), 7, 40);
        load.abandon(&abandoned, stale);

        // When the abandoned one's result lands.
        load.complete(abandoned.clone(), stale, 7, 40, lines("late"));

        // Then the other session's request is still tracked, so its result will
        // not be mistaken for a late arrival.
        assert!(
            load.in_flight_matches(&live, 7, 40),
            "abandoning one session must not clear another's in-flight request"
        );
    }

    #[rstest::rstest]
    fn a_result_from_another_session_is_kept() {
        // Given a session with a live request at a later generation.
        let live = SessionId::new();
        let other = SessionId::new();
        let mut load = PreviewLoad::default();
        let live_generation = load.request(live.clone(), 7, 40);
        let earlier = live_generation.saturating_sub(1);

        // When the other session's earlier-numbered result lands.
        let accepted = load.complete(other.clone(), earlier, 7, 40, lines("other"));

        // Then it is kept: generations count every request the sidebar has ever
        // made, so a smaller number is not evidence of staleness across
        // sessions. Only supersession *within* a session makes a result stale.
        assert!(
            accepted,
            "one session's generation must not reject another session's result"
        );
    }

    #[rstest::rstest]
    fn a_result_superseded_by_a_newer_request_is_discarded() {
        // Given a request in flight, superseded by a second for the same session.
        let session_id = SessionId::new();
        let mut load = PreviewLoad::default();
        load.request(session_id.clone(), 7, 40);
        let current = load.request(session_id.clone(), 8, 40);

        // When the first request's result arrives after the second was armed.
        let accepted = load.complete(
            session_id.clone(),
            current.saturating_sub(1),
            7,
            40,
            lines("stale"),
        );

        // Then it is refused: it was built against content the newer request
        // already replaced, and writing it would strand the newer one.
        assert!(!accepted, "a superseded result must not be stored");
    }
}

/// The theme-change guard: a render in flight during a reset must not write
/// lines carrying the theme the user just replaced.
#[cfg(test)]
mod preview_reset_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::preview_load_tests::lines;
    use super::*;

    #[rstest::rstest]
    fn a_late_result_after_a_reset_is_discarded() {
        // Given a request in flight when the theme changes.
        let session_id = SessionId::new();
        let mut load = PreviewLoad::default();
        let generation = load.request(session_id.clone(), 7, 40);
        load.reset();

        // When that render's result lands.
        let accepted = load.complete(session_id.clone(), generation, 7, 40, lines("old theme"));

        // Then it is refused, because its lines carry the replaced theme.
        assert!(!accepted, "a pre-reset result must be discarded");
    }

    #[rstest::rstest]
    fn a_result_after_a_reset_arms_a_new_request_is_accepted() {
        // Given a reset, followed by a fresh request.
        let session_id = SessionId::new();
        let mut load = PreviewLoad::default();
        load.request(session_id.clone(), 7, 40);
        load.reset();
        let generation = load.request(session_id.clone(), 7, 40);

        // When that request's result lands.
        let accepted = load.complete(session_id.clone(), generation, 7, 40, lines("new theme"));

        // Then it is kept — the reset's floor must not reject its own epoch.
        assert!(accepted, "a post-reset result must be stored");
        assert!(load.cached(&session_id, 7, 40).is_some());
    }

    #[rstest::rstest]
    fn a_reset_drops_cached_previews() {
        // Given a session with a rendered preview.
        let session_id = SessionId::new();
        let mut load = PreviewLoad::default();
        let generation = load.request(session_id.clone(), 7, 40);
        load.complete(session_id.clone(), generation, 7, 40, lines("hello"));

        // When the theme changes.
        load.reset();

        // Then the old-theme lines are gone, so the next cursor move re-renders.
        assert!(
            load.cached(&session_id, 7, 40).is_none(),
            "a reset must drop lines carrying the replaced theme"
        );
    }
}

/// The cache's bound, and what it means for which previews survive navigation.
#[cfg(test)]
mod preview_cache_bound_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::preview_freshness_tests::ready;
    use super::*;

    /// Caches `count` distinct sessions, oldest first, and returns their ids.
    fn fill(load: &mut PreviewLoad, count: usize) -> Vec<SessionId> {
        let ids: Vec<SessionId> = std::iter::repeat_with(SessionId::new).take(count).collect();
        for id in &ids {
            ready(load, id, 7, 40, "hello");
        }
        ids
    }

    #[rstest::rstest]
    fn the_cache_evicts_beyond_its_bound() {
        // Given one more session cached than the cache holds.
        let mut load = PreviewLoad::default();
        let ids = fill(&mut load, PREVIEW_CACHE_CAPACITY + 1);
        let evicted = ids.first().expect("a session was cached");

        // Then the least-recently-used session is gone.
        assert!(
            load.cached(evicted, 7, 40).is_none(),
            "the cache must not grow past its bound"
        );
    }

    #[rstest::rstest]
    fn the_cache_keeps_the_most_recent_within_its_bound() {
        // Given one more session cached than the cache holds.
        let mut load = PreviewLoad::default();
        let ids = fill(&mut load, PREVIEW_CACHE_CAPACITY + 1);
        let newest = ids.last().expect("a session was cached");

        // Then the newest session is still served.
        assert!(
            load.cached(newest, 7, 40).is_some(),
            "the most recent preview must survive the bound"
        );
    }

    #[rstest::rstest]
    fn a_touched_preview_outlives_an_untouched_one() {
        // Given a full cache, with its oldest entry refreshed as just-used.
        let mut load = PreviewLoad::default();
        let ids = fill(&mut load, PREVIEW_CACHE_CAPACITY);
        let refreshed = ids.first().expect("a session was cached");
        load.touch(refreshed);

        // When one more session is cached, overflowing the bound.
        ready(&mut load, &SessionId::new(), 7, 40, "hello");

        // Then the touched session survives, and the session it displaced does
        // not. Without the touch, recency would be insertion order and the
        // session the user is actually looking at would be the first to go.
        assert!(
            load.cached(refreshed, 7, 40).is_some(),
            "a refreshed preview must not be the next one evicted"
        );
        let untouched = ids.get(1).expect("a session was cached");
        assert!(
            load.cached(untouched, 7, 40).is_none(),
            "the least-recently-used preview must be the one evicted"
        );
    }

    #[rstest::rstest]
    fn caching_a_session_twice_does_not_consume_two_slots() {
        // Given a cache holding a session, which is then re-rendered.
        let mut load = PreviewLoad::default();
        let ids = fill(&mut load, PREVIEW_CACHE_CAPACITY);
        let revisited = ids.first().expect("a session was cached");
        ready(&mut load, revisited, 7, 40, "hello again");

        // When one more session is cached, overflowing the bound.
        ready(&mut load, &SessionId::new(), 7, 40, "hello");

        // Then the re-rendered session is still held, so a re-cache did not
        // evict a neighbour to make room for a second copy of itself.
        assert!(
            load.cached(revisited, 7, 40).is_some(),
            "re-caching a session must replace its entry, not duplicate it"
        );
    }
}
