//! The per-frame write pass — slice-registered pre-render hooks.
//!
//! The render loop needs one write lock on shared state per frame, and
//! the bookkeeping that has to run under it is mostly *slices'*
//! business: the sidebar measures its preview geometry, the terminal
//! overlay sizes its pty, the chat input re-wraps its draft. Composition
//! used to call those functions by name, which meant the TUI layer had
//! to import every slice whose numbers it happened to need.
//!
//! This registry moves the *call sites* out of the TUI layer. A slice
//! registers a [`PreRenderHook`] at activation; the render pass takes
//! the write lock once and runs every hook under it.
//!
//! ## Why the state type is a parameter
//!
//! A hook's first argument is `&mut S`, the application state. This
//! crate cannot name `AppState` — it is the extraction seam and depends
//! only on `jinn-theme` and `ratatui`, never on `jinn-kernel`. So the
//! hook list is generic over the state type and the kernel instantiates
//! it at `AppState`.
//!
//! The list itself is reached through [`Slices`](crate::Slices), which
//! type-erases it behind `dyn Any` — the same trick the draw registry
//! uses for slice-owned elements. That keeps the seam invisible: a
//! slice's `activate` asks its host for the hook list, exactly as it
//! asks for its cells, and neither the host's constructor nor the
//! `Services` struct grows a field.
//!
//! ## Hooks return their publishes
//!
//! A hook does not publish. It returns the closures it wants sent, in
//! order, and the render pass sends them on its own bridge. This keeps
//! the bridge — a kernel type — out of the hook signature, and keeps
//! the publish order identical to the call order.

use std::fmt;
use std::sync::Arc;

use parking_lot::RwLock;
use ratatui::layout::Rect;

use crate::route::PublishClosure;
use crate::slices::Slices;

/// The chat-layout rects a pre-render hook may need.
///
/// The render pass computes the layout; a hook only reads the rects
/// out of it. `chat` is `None` in a full-width tab, where the sidebar
/// column and the main column's input box do not exist — a hook that
/// needs them stands down, which is what the layout match it replaces
/// did.
#[derive(Debug, Clone, Copy)]
pub struct ChatRects {
    /// The main column: the chat log, the input box, the status bar.
    pub main: Rect,
    /// The sidebar column, right of the chat border.
    pub sidebar: Rect,
    /// The message composer at the bottom of the main column.
    pub input: Rect,
}

/// Everything a pre-render hook is told about the frame it precedes.
///
/// Deliberately data, not a render context: a hook runs under a *write*
/// lock before the read context exists, and it must not be able to draw.
#[derive(Debug, Clone, Copy)]
pub struct PreRenderCtx<'a> {
    /// The whole frame area.
    pub frame_area: Rect,
    /// The chat-layout rects, or `None` in a full-width tab.
    pub chat: Option<ChatRects>,
    /// The live configuration layer, read at the point of use so a
    /// reload is visible without re-wiring.
    pub config: &'a crate::ConfigLayer,
}

/// A slice-registered pre-render hook: writes its own bookkeeping into
/// `state` and returns the closures it wants published, in order.
///
/// Returning `Vec<PublishClosure>` rather than publishing directly is
/// what keeps the bridge out of this signature; an empty vector means
/// the hook only wrote state.
pub type PreRenderHook<S> = Arc<dyn Fn(&mut S, &PreRenderCtx<'_>) -> Vec<PublishClosure> + Send + Sync>;

/// A hook wrapped for `Debug` (closures are not `Debug`).
struct HookEntry<S: 'static>(PreRenderHook<S>);

impl<S: 'static> Clone for HookEntry<S> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<S: 'static> fmt::Debug for HookEntry<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PreRenderHook(..)")
    }
}

/// The registry of slice-registered pre-render hooks, in call order.
///
/// Order is the point: the hooks that ran inline in the render pass
/// ran in a specific order, and two of them write the same state, so
/// this is an ordered list, not a map. Registration appends; a slice
/// that must run earlier than another is wired earlier in the boot list.
#[derive(Debug)]
pub struct PreRenderHooks<S: 'static> {
    hooks: Arc<RwLock<Vec<HookEntry<S>>>>,
}

impl<S: 'static> Clone for PreRenderHooks<S> {
    /// A handle to the same list, not a copy of it — several slices hold
    /// one and all of their pushes land in one call order.
    fn clone(&self) -> Self {
        Self {
            hooks: Arc::clone(&self.hooks),
        }
    }
}

impl<S: 'static> Default for PreRenderHooks<S> {
    fn default() -> Self {
        Self {
            hooks: Arc::new(RwLock::new(Vec::new())),
        }
    }
}

impl<S: 'static> PreRenderHooks<S> {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends `hook` to the call order.
    pub fn push(&self, hook: PreRenderHook<S>) {
        self.hooks.write().push(HookEntry(hook));
    }

    /// Runs every hook in registration order against `state`, collecting
    /// the closures they want published, in the same order.
    ///
    /// Takes the state by mutable reference once: the caller holds the
    /// write lock across the whole pass, so no hook re-acquires it and
    /// no hook can block on it.
    pub fn run(&self, state: &mut S, ctx: &PreRenderCtx<'_>) -> Vec<PublishClosure> {
        let hooks = self.hooks.read().clone();
        let mut publishes = Vec::new();
        for hook in hooks {
            publishes.extend((hook.0)(state, ctx));
        }
        publishes
    }

    /// How many hooks are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.hooks.read().len()
    }

    /// Whether no hook is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The slot key the shared pre-render hook list is stored under.
///
/// The list is a [`Slices`] cell rather than a new field on `Services`
/// or on [`SliceHost`](crate::SliceHost): the cell registry already
/// mints one shared handle to a value several slices push into, which
/// is exactly what a hook list is. Keeping it there means the seam
/// costs no new constructor argument anywhere — including in the 39
/// `SliceHost::new` call sites and the 12 `Services` literals that a
/// new field would have had to touch.
#[must_use]
pub fn pre_render_hooks_slot() -> crate::slices::SlotKey {
    crate::slices::SlotKey::builtin("jinn", "pre-render-hooks")
}

impl Slices {
    /// The shared pre-render hook list, resolved at state type `S`.
    ///
    /// The first caller registers the list; every later caller — the
    /// next slice's `activate`, the render pass — resolves a handle to
    /// that same list. `None` means the slot holds a list for a
    /// *different* state type, which no workspace path produces.
    #[must_use]
    pub fn pre_render_hooks<S: Send + Sync + 'static>(&self) -> Option<PreRenderHooks<S>> {
        let cell = self.get_or_register(&pre_render_hooks_slot(), PreRenderHooks::<S>::new())?;
        let hooks = cell.read();
        // Clone the handle out from under the guard; the list itself is
        // shared, so the returned handle keeps observing later pushes.
        Some(hooks.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::ChatRects;
    use super::PreRenderCtx;
    use super::PreRenderHooks;
    use crate::ConfigLayer;

    #[derive(Default, Debug, PartialEq, Eq)]
    struct FakeState {
        order: Vec<&'static str>,
    }

    fn ctx(config: &ConfigLayer, chat: Option<ChatRects>) -> PreRenderCtx<'_> {
        PreRenderCtx {
            frame_area: ratatui::layout::Rect::new(0, 0, 80, 24),
            chat,
            config,
        }
    }

    fn rect() -> ratatui::layout::Rect {
        ratatui::layout::Rect::new(0, 0, 10, 10)
    }

    #[rstest::rstest]
    fn hooks_run_in_registration_order() {
        // Given two hooks that each record themselves.
        let hooks: PreRenderHooks<FakeState> = PreRenderHooks::new();
        hooks.push(Arc::new(|state: &mut FakeState, _| {
            state.order.push("first");
            Vec::new()
        }));
        hooks.push(Arc::new(|state: &mut FakeState, _| {
            state.order.push("second");
            Vec::new()
        }));
        let mut state = FakeState::default();
        let config = crate::empty_config_layer();

        // When running the pass.
        let publishes = hooks.run(&mut state, &ctx(config, None));

        // Then the hooks ran in registration order.
        assert_eq!(state.order, vec!["first", "second"]);
        // And neither published.
        assert!(publishes.is_empty());
    }

    #[rstest::rstest]
    fn hook_receives_the_chat_rects_when_the_layout_has_them() {
        // Given a hook that records the chat rects it was handed.
        let hooks: PreRenderHooks<FakeState> = PreRenderHooks::new();
        let expected = ChatRects {
            main: rect(),
            sidebar: rect(),
            input: rect(),
        };
        hooks.push(Arc::new(
            move |state: &mut FakeState, ctx: &PreRenderCtx<'_>| {
                state.order.push(if ctx.chat.is_some() { "chat" } else { "tab" });
                Vec::new()
            },
        ));
        let mut state = FakeState::default();
        let config = crate::empty_config_layer();

        // When running the pass for a chat layout and for a tab layout.
        hooks.run(&mut state, &ctx(config, Some(expected)));
        hooks.run(&mut state, &ctx(config, None));

        // Then the hook saw a chat layout and then a tab layout.
        assert_eq!(state.order, vec!["chat", "tab"]);
    }

    #[rstest::rstest]
    fn hook_writes_reach_the_caller_state() {
        // Given a hook that writes a value into the state it is handed.
        let hooks: PreRenderHooks<FakeState> = PreRenderHooks::new();
        hooks.push(Arc::new(|state: &mut FakeState, _| {
            state.order.push("written");
            Vec::new()
        }));
        let mut state = FakeState::default();
        let config = crate::empty_config_layer();

        // When running the pass.
        hooks.run(&mut state, &ctx(config, None));

        // Then the write is visible to the caller.
        assert_eq!(state.order, vec!["written"]);
    }

    #[rstest::rstest]
    fn empty_registry_runs_nothing() {
        // Given a registry with no hooks.
        let hooks: PreRenderHooks<FakeState> = PreRenderHooks::new();
        let mut state = FakeState::default();
        let config = crate::empty_config_layer();

        // When running the pass.
        let publishes = hooks.run(&mut state, &ctx(config, None));

        // Then nothing ran and nothing published.
        assert!(hooks.is_empty());
        assert_eq!(hooks.len(), 0);
        assert!(publishes.is_empty());
        assert!(state.order.is_empty());
    }
}
