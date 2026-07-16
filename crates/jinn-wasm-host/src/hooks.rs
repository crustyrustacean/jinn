//! Hook firing via runtime export lookup.
//!
//! The host never hard-codes which hooks a plugin implements. It probes the
//! instance's exports by name: if present, call it; if absent, skip it
//! (optional-hook semantics). This is what lets plugins define their own
//! hooks (e.g. `on-enrich`) that the host has no compile-time knowledge of.
//!
//! # Well-known vs plugin-defined hooks
//!
//! The `hooks` interface in WIT declares the *well-known* hooks
//! (`on-turn-end`, `on-chat-input-badges-render`, …). These are typed: the
//! generated `Hooks` trait has a method per hook, and the host calls them
//! through the typed bindings.
//!
//! Plugin-defined hooks (e.g. `on-enrich`) are NOT in the interface — the
//! plugin declares a keybind whose `action` is an arbitrary string, and the
//! host fires it by looking up the export by name at runtime. The hook's
//! `trigger-ctx` is still a typed WIT record; only the *name* is a string.
//!
//! # Sync vs async
//!
//! Sync hooks (`badges`, `keybind-trigger`, `submit-intercept`) run on the
//! render-thread store set, synchronously. Async hooks (lifecycle, LLM,
//! plugin-defined) run on the async host-thread store set, via `call_async`.

use wasmtime::AsContextMut;

use crate::store::{StoreKind, StoreSet};

/// Error firing a hook.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub struct HookFireError;

/// Whether a named export exists on an instance.
///
/// Used to implement optional-hook semantics: the host probes for an export
/// before calling, skipping absent ones. Both well-known and plugin-defined
/// hooks go through this probe.
#[must_use]
pub fn export_exists(
    storeset: &mut StoreSet,
    plugin_name: &str,
    instance_id: &jinn_core_types::PluginInstanceId,
    hook_name: &str,
) -> bool {
    let Some(inst) = storeset.get_mut(plugin_name, instance_id) else {
        return false;
    };
    inst.with(|store, instance| {
        instance
            .get_func(store.as_context_mut(), hook_name)
            .is_some()
    })
}

// Anchor the StoreKind import: sync vs async firing is selected by this.
#[allow(dead_code)]
fn _kind_anchor(k: StoreKind) {
    let _ = k;
}

/// Fire a plugin-defined async hook by runtime export name.
///
/// `hook_name` is the arbitrary string the plugin declared (e.g.
/// `"on-enrich"`). The host looks up the export by name; if absent, returns
/// `Ok(None)` (skip — optional-hook semantics). If present, the export is
/// invoked with no args (plugin-defined hooks read their ctx from host
/// imports or a prior ctx-setting call, matching the Lua pattern where the
/// action string is the only out-of-band datum).
///
/// This is the dynamic-lookup path. Well-known hooks use the typed
/// bindgen accessors (see Phase 3 wiring); plugin-defined hooks — whose
/// names the host has no compile-time knowledge of — go through here.
///
/// # Errors
///
/// Returns an error only if the export exists but the call traps.
pub async fn fire_async_by_name(
    storeset: &mut StoreSet,
    plugin_name: &str,
    instance_id: &jinn_core_types::PluginInstanceId,
    hook_name: &str,
) -> Result<Option<()>, error_stack::Report<HookFireError>> {
    let Some(inst) = storeset.get_mut(plugin_name, instance_id) else {
        return Ok(None);
    };

    let exists = inst.with(|store, instance| instance.get_func(&mut *store, hook_name).is_some());
    if !exists {
        return Ok(None);
    }

    inst.with_async(|store, instance| async move {
        use error_stack::{Report, ResultExt};
        let func = instance
            .get_func(&mut *store, hook_name)
            .ok_or_else(|| Report::new(HookFireError))
            .attach(hook_name.to_owned())
            .attach("hook export vanished between probe and call")?;
        let mut results: Vec<wasmtime::component::Val> = Vec::new();
        func.call_async(&mut *store, &[], &mut results)
            .await
            .map_err(|e| Report::new(HookFireError).attach(e.to_string()))
            .attach(hook_name.to_owned())
            .attach("calling plugin-defined async hook")?;
        Ok(Some(()))
    })
    .await
}
