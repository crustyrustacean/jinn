//! Slice-registered overlay renderers — the dynamic-scope overlay pass.
//!
//! One entry per overlay slice (e.g. the quake bar); written at slice
//! activation (single writer, read-only at render time). Cheap to
//! clone: shared behind `Arc`.
//!
//! Generic over the render context: the kernel instantiates
//! `OverlayViews<RenderCtx>` where the context carries app state; a
//! slice crate instantiates it over its own render-context type. The
//! registry itself only requires the closure to be callable with
//! `(frame, rect, &C)`.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::SliceScopeId;

/// A slice-registered overlay renderer: draws the slice's overlay into
/// `area` for one frame, with the caller's render context.
pub type OverlayViewFn<C> = Arc<dyn Fn(&mut Frame<'_>, Rect, &C) + Send + Sync>;

/// An overlay fn wrapped for `Debug` (closures are not `Debug`).
#[derive(Clone)]
struct OverlayEntry<C: 'static>(OverlayViewFn<C>);

impl<C: 'static> std::fmt::Debug for OverlayEntry<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OverlayViewFn(..)")
    }
}

/// The registry of dynamic-scope overlay renderers.
#[derive(Clone, Debug)]
pub struct OverlayViews<C: 'static> {
    views: Arc<RwLock<HashMap<SliceScopeId, OverlayEntry<C>>>>,
}

impl<C: 'static> Default for OverlayViews<C> {
    fn default() -> Self {
        Self {
            views: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl<C: 'static> OverlayViews<C> {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the renderer for `scope`, replacing any previous one.
    pub fn register(&self, scope: SliceScopeId, view: OverlayViewFn<C>) {
        self.views.write().insert(scope, OverlayEntry(view));
    }

    /// Returns the renderer registered for `scope`, if any.
    #[must_use]
    pub fn view(&self, scope: &SliceScopeId) -> Option<OverlayViewFn<C>> {
        self.views.read().get(scope).map(|entry| entry.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::OverlayViews;
    use crate::SliceScopeId;

    #[rstest::rstest]
    #[test]
    fn register_then_resolve_roundtrips_scope() {
        // Given an empty overlay registry.
        let views: OverlayViews<u8> = OverlayViews::new();
        let scope = SliceScopeId::new("test-slice", "overlay");

        // When registering a renderer for the scope.
        views.register(
            scope.clone(),
            std::sync::Arc::new(|_, _, ctx| {
                let _ = ctx;
            }),
        );

        // Then the registry resolves it back for that scope.
        assert!(views.view(&scope).is_some());
        // And an unregistered scope resolves nothing.
        assert!(
            views
                .view(&SliceScopeId::new("other-slice", "overlay"))
                .is_none()
        );
    }
}
