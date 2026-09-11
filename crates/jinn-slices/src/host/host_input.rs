//! Input-hook staging: slices declare editing-intent interceptors at
//! activation; composition installs them into the kernel's route
//! table afterwards.
//!
//! Staging (rather than installing directly) keeps activation a pure
//! registration sequence and gives composition one place to observe
//! every hook a slice registered — the seam a future WASM guest host
//! will reuse when declaring hooks from manifest messages.

use std::sync::Arc;

use crate::SliceScopeId;
use crate::route::EditIntent;
use crate::route::InputHook;
use crate::route::RouteResult;

/// A staged hook: the scope it serves and the serving closure.
#[derive(Clone, Default)]
pub struct HookRegistry {
    hooks: Vec<(SliceScopeId, InputHook)>,
}

impl std::fmt::Debug for HookRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookRegistry")
            .field("scopes", &self.scopes())
            .finish()
    }
}

/// A dispatch request the kernel handler makes against an installed
/// hook. Kept as a named type so the contract reads in signatures.
pub struct HookRequest<'a> {
    /// The edit intent the focused scope's surface may serve.
    pub intent: &'a EditIntent,
}

impl HookRegistry {
    /// Stages a hook for `scope`.
    pub fn register(&mut self, scope: SliceScopeId, hook: InputHook) {
        self.hooks.push((scope, hook));
    }

    /// The scopes with staged hooks, in staging order.
    #[must_use]
    pub fn scopes(&self) -> Vec<SliceScopeId> {
        self.hooks.iter().map(|(scope, _)| scope.clone()).collect()
    }

    /// Installs every staged hook through `install` and clears the
    /// staging area.
    pub fn install<I>(mut self, mut install: I)
    where
        I: FnMut(SliceScopeId, InputHook),
    {
        for (scope, hook) in self.hooks.drain(..) {
            install(scope, hook);
        }
    }
}

/// Runs a single hook against an edit intent (the handler's
/// per-keystroke entry point).
#[must_use]
pub fn serve(hook: &InputHook, intent: &EditIntent) -> Option<RouteResult> {
    hook(intent)
}

/// Wraps a plain serving closure into the registry's hook type.
#[must_use]
pub fn hook<F>(serve: F) -> InputHook
where
    F: Fn(&EditIntent) -> Option<RouteResult> + Send + Sync + 'static,
{
    Arc::new(serve)
}

#[cfg(test)]
mod tests {
    use super::HookRegistry;
    use super::hook;
    use super::serve;
    use crate::SliceScopeId;
    use crate::route::EditIntent;
    use crate::route::RouteResult;

    #[rstest::rstest]
    #[test]
    fn install_drains_staged_hooks_in_order() {
        // Given a registry with two staged hooks.
        let mut registry = HookRegistry::default();
        registry.register(
            SliceScopeId::new("a", "main"),
            hook(|_| Some(RouteResult::empty())),
        );
        registry.register(SliceScopeId::new("b", "main"), hook(|_| None));

        // When installing them into a sink.
        let mut installed = Vec::new();
        registry.install(|scope, _| installed.push(scope));

        // Then both arrived, in staging order.
        assert_eq!(
            installed,
            vec![
                SliceScopeId::new("a", "main"),
                SliceScopeId::new("b", "main")
            ]
        );
    }

    #[rstest::rstest]
    #[test]
    fn serve_dispatches_request_into_hook() {
        // Given a hook that serves only backward deletes.
        let h =
            hook(|intent| matches!(intent, EditIntent::DeleteBackward).then(RouteResult::empty));

        // When serving a matching intent.
        let hit = serve(&h, &EditIntent::DeleteBackward);

        // Then it produced a result, and a non-matching intent misses.
        assert!(hit.is_some());
        assert!(serve(&h, &EditIntent::InsertChar('x')).is_none());
    }
}
