//! Standardized render context for the TUI render path.
//!
//! [`RenderCtx`] wraps a shared reference to [`AppState`] plus the
//! slices registry, and is threaded through every render function. It
//! provides a single, extensible context type that can grow to hold
//! command sinks, or other capabilities without changing function
//! signatures. Slice renderers resolve their cells through
//! [`RenderCtx::slices`] instead of reading `FrontendState` fields.

use crate::common::app_state::AppState;
use jinn_context_curation_msg::prune_report;
use jinn_slices::AppFact;
use jinn_slices::OverlayViewFn;
use jinn_slices::OverlayViews;
use jinn_slices::Slices;
use jinn_slices::render_facts::RenderFacts as SliceFacts;

/// Render context passed to every render function.
///
/// Contains read-only access to application state and the slice
/// registry. Constructed once per frame in the top-level `render()`
/// function and passed through the entire render tree.
pub struct RenderCtx<'a> {
    /// Read-only application state.
    pub state: &'a AppState,
    /// The slices registry: slice renderers resolve read handles here.
    pub slices: &'a Slices,
    /// Slice-registered overlay renderers for dynamic scopes (the quake
    /// bar). Overlay slices register at activation; an unregistered
    /// scope renders nothing.
    pub overlay_views: &'a OverlayViews<SliceFacts>,
    /// The configuration layer. Render-path consumers read config the
    /// same way every other consumer does — a live handle, at the point
    /// of use — rather than through a cache seeded elsewhere.
    ///
    /// A cheap cloneable handle, not a borrowed snapshot: a frame that
    /// straddles a `reload` sees the new value, and holding it costs an
    /// `Arc` bump rather than a document clone.
    pub config: &'a jinn_config::ConfigLayer,
}

impl<'a> RenderCtx<'a> {
    /// Creates a new render context wrapping the given state reference,
    /// slices registry, and overlay-view registry.
    pub fn new(
        state: &'a AppState,
        slices: &'a Slices,
        overlay_views: &'a OverlayViews<SliceFacts>,
        config: &'a jinn_config::ConfigLayer,
    ) -> Self {
        Self {
            state,
            slices,
            overlay_views,
            config,
        }
    }

    /// A context over an empty configuration layer, for tests and for any
    /// caller that has no layer at hand.
    ///
    /// Every section reads as its default through this, so a test that
    /// does not care about configuration can build a context without one.
    #[must_use]
    pub fn new_with_default_config(
        state: &'a AppState,
        slices: &'a Slices,
        overlay_views: &'a OverlayViews<SliceFacts>,
    ) -> Self {
        Self::new(state, slices, overlay_views, empty_config_layer())
    }

    /// Returns the overlay renderer registered for a dynamic scope, if
    /// any. Overlay slices register at activation; an unregistered scope
    /// renders nothing.
    #[must_use]
    pub fn overlay_view(
        &self,
        scope: &jinn_slices::SliceScopeId,
    ) -> Option<OverlayViewFn<SliceFacts>> {
        self.overlay_views.view(scope)
    }

    /// Builds the slice-facing facts context for one frame: the same
    /// theme the app state carries, the live registry, and the
    /// session-facts composition seeds each frame (prune accumulator,
    /// prune report, lifecycle label). Slice overlay renderers receive
    /// this instead of `&RenderCtx` so their crates stay kernel-free.
    #[must_use]
    pub fn facts(&self) -> jinn_slices::RenderFacts {
        let session = self.state.active_session();
        let report = prune_report(session.history());
        let mut facts = SliceFacts::new(self.state.frontend.theme.clone(), self.slices);
        // The term overlay's facts: the mirror key, the capture flag, and
        // the configured toggle key (the border hint's capture glyph).
        // Consumed by the term slice's overlay renderer (`term:capture`
        // styling and hints); absent facts degrade chrome, never panic.
        let capturing = matches!(
            self.state.frontend.scope(),
            jinn_slices::FocusScope::Dynamic(id)
                if id == jinn_term_msg::control_scope()
        );
        let toggle_key = self
            .config
            .read::<jinn_preferences_config::schemas::term::InteractiveTermPrefs>()
            .control_toggle_key
            .clone();
        facts.set_facts([
            AppFact {
                key: "session.prune-pending",
                value: format!(
                    "Prune ctx pending: {} tok",
                    session.accumulated_overrides_total()
                ),
            },
            AppFact {
                key: "session.prune-pruned",
                value: format!(
                    "Prune ctx pruned: {} tok ({} entries)",
                    report.tokens, report.entries
                ),
            },
            AppFact {
                key: "session.cwd",
                value: session.cwd().display().to_string(),
            },
            AppFact {
                key: "session.lifecycle",
                value: match session.lifecycle_name() {
                    None => "Lifecycle: <none>".to_owned(),
                    Some(name) => {
                        format!("Lifecycle: {name} ({})", session.lifecycle_script_state())
                    }
                },
            },
            AppFact {
                key: jinn_term_msg::overlay_facts::SESSION_ID,
                value: session.session_id().to_string(),
            },
            AppFact {
                key: jinn_term_msg::overlay_facts::CAPTURING,
                value: if capturing { "1" } else { "0" }.to_owned(),
            },
            AppFact {
                key: jinn_term_msg::overlay_facts::TOGGLE_KEY,
                value: toggle_key,
            },
        ]);
        facts
    }
}

/// A process-lifetime configuration layer with nothing in it, for tests
/// that reach a config-taking function without caring about config.
///
/// Every section reads as its default through it. Backed by a
/// `OnceLock` so a test does not allocate a document per call.
///
/// # Panics
///
/// Panics if the shared empty layer cannot be constructed. That can only
/// fail if an empty document stops parsing, which is a build-time
/// invariant of the layer rather than anything a caller can cause.
#[cfg(any(test, feature = "test-harness"))]
#[must_use]
#[expect(
    clippy::expect_used,
    reason = "an empty document always parses; failure is a broken invariant, not a caller error"
)]
pub fn empty_config_layer() -> &'static jinn_config::ConfigLayer {
    static EMPTY: std::sync::OnceLock<jinn_config::ConfigLayer> = std::sync::OnceLock::new();
    EMPTY.get_or_init(|| {
        jinn_config::ConfigLayer::load(std::sync::Arc::new(
            jinn_config::InMemoryConfigStorage::default(),
        ))
        .expect("an empty document always loads")
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test code")]

    use super::RenderCtx;
    use crate::common::app_state::AppState;
    use crate::protocol::ChangeSource;
    use crate::protocol::ChatEntry;
    use crate::protocol::ChatEntryId;
    use crate::protocol::ContextOverride;
    use jinn_slices::OverlayViews;
    use jinn_slices::Slices;

    fn ctx_for<'a>(
        state: &'a AppState,
        slices: &'a Slices,
        views: &'a OverlayViews<jinn_slices::render_facts::RenderFacts>,
    ) -> RenderCtx<'a> {
        RenderCtx::new_with_default_config(state, slices, views)
    }

    fn worker_prune(entry: &mut ChatEntry) {
        entry.apply_context_override(
            ContextOverride::ForcedExclude,
            ChangeSource::Worker {
                name: "edit_read".to_owned(),
            },
        );
    }

    fn facts_via(state: &AppState, slices: &Slices) -> jinn_slices::render_facts::RenderFacts {
        let views = OverlayViews::new();
        ctx_for(state, slices, &views).facts()
    }

    #[rstest::rstest]
    #[test]
    fn facts_seed_the_prune_pending_label_from_the_accumulator() {
        // Given an app state whose session has a buffered prune mutation.
        let mut state = AppState::default();
        state.active_session_mut().route_override(
            ChatEntryId::new(),
            ContextOverride::ForcedExclude,
            ChangeSource::User,
            120,
        );
        let slices = Slices::new();

        // When building the facts context.
        let facts = facts_via(&state, &slices);

        // Then the prune-pending fact carries the trunk-format label.
        assert_eq!(
            facts.fact("session.prune-pending"),
            Some("Prune ctx pending: 120 tok")
        );
    }

    #[rstest::rstest]
    #[test]
    fn facts_seed_the_prune_pruned_label_from_the_report() {
        // Given an app state whose history holds a worker-pruned entry.
        let mut state = AppState::default();
        let mut entry = ChatEntry::user("big tool output");
        entry.token_count = Some(55);
        worker_prune(&mut entry);
        state.active_session_mut().push_entry(entry);
        let slices = Slices::new();

        // When building the facts context.
        let facts = facts_via(&state, &slices);

        // Then the prune-pruned fact carries the trunk-format label.
        assert_eq!(
            facts.fact("session.prune-pruned"),
            Some("Prune ctx pruned: 55 tok (1 entries)")
        );
    }

    #[rstest::rstest]
    #[test]
    fn facts_seed_the_lifecycle_label_for_blank_lifecycles() {
        // Given an app state with no lifecycle on the active session.
        let state = AppState::default();
        let slices = Slices::new();

        // When building the facts context.
        let facts = facts_via(&state, &slices);

        // Then the lifecycle fact shows the trunk "<none>" label.
        assert_eq!(facts.fact("session.lifecycle"), Some("Lifecycle: <none>"));
    }

    #[rstest::rstest]
    #[test]
    fn facts_seed_the_lifecycle_label_with_script_state() {
        // Given an app state whose session was created by a lifecycle.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .set_lifecycle_name(Some("review".to_owned()));
        let slices = Slices::new();

        // When building the facts context.
        let facts = facts_via(&state, &slices);

        // Then the lifecycle fact names the lifecycle and its script state.
        assert_eq!(
            facts.fact("session.lifecycle"),
            Some("Lifecycle: review (nothing_ran)")
        );
    }
}
