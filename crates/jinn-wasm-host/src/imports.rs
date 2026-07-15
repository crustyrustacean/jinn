//! Host imports — what a plugin calls.
//!
//! These implement the `host` interface from `wit/jinn.wit`. Each import is
//! registered into a `wasmtime::component::Linker<StoreState>`. The plugin
//! invokes them (possibly awaiting, for async ones); the host services them.
//!
//! Three categories:
//!
//! 1. **`emit(cmd)`** — typed WIT `command` → domain `BusMessage` → bus.
//!    Replaces the old Lua `dispatch_verb`.
//! 2. **`request-llm-oneshot` / `create-session`** — async imports. Calling
//!    one suspends the component (wasmtime async) until the host's `Future`
//!    resolves, then resumes. This is the coroutine idiom.
//! 3. **`cancel-task(name)`** — cancels a named in-flight async request.
//! 4. **Bag accessors** — `get/set-plugin-data`, `get/set-global-data`.
//!    Postcard-blind; the host never inspects the bytes.
//!
//! The concrete behaviour (how `emit` reaches the bus, how an LLM one-shot
//! is driven) is supplied by callbacks injected at build time, exactly as the
//! old `PluginSystem::build` took `command_dispatcher` + `request_handler`.
//! This keeps the host crate decoupled from domain internals.
//!
//! The generated `Host` trait (from `bindings::jinn::plugin::host`) is
//! implemented on `StoreState` in Phase 3, where the callbacks are wired to
//! the real domain services. Phase 2 establishes the callback types and the
//! linker-registration entry point shape.

use std::sync::Arc;

use wasmtime::component::Linker;

use crate::bindings::command::{Command, CreateSessionReq, CreateSessionResp, LlmOneshotReq, LlmResp};
use crate::store::InstanceCtx;

/// Callback that services a plugin `emit(cmd)`.
///
/// Receives the typed WIT command and the instance identity, and dispatches
/// it to the bus. Injected by the wiring layer; the host crate stays
/// domain-agnostic.
pub type EmitCallback =
    Arc<dyn Fn(&str, &Command, &InstanceCtx) + Send + Sync>;

/// Callback that services an async `request-llm-oneshot`.
///
/// Returns the typed response (the plugin deserializes via its PDK codec).
pub type LlmOneshotCallback = Arc<
    dyn Fn(
            &InstanceCtx,
            &LlmOneshotReq,
        ) -> futures::future::BoxFuture<'static, Result<LlmResp, String>>
        + Send
        + Sync,
>;

/// Callback that services an async `create-session`.
pub type CreateSessionCallback = Arc<
    dyn Fn(
            &InstanceCtx,
            &CreateSessionReq,
        ) -> futures::future::BoxFuture<'static, Result<CreateSessionResp, String>>
        + Send
        + Sync,
>;

/// Callback that cancels a named in-flight task.
pub type CancelTaskCallback = Arc<dyn Fn(&str) + Send + Sync>;

/// The injected host behaviours. Held in an `Arc` shared by all stores.
#[derive(Clone)]
pub struct HostImports {
    pub emit: EmitCallback,
    pub llm_oneshot: LlmOneshotCallback,
    pub create_session: CreateSessionCallback,
    pub cancel_task: CancelTaskCallback,
}


impl std::fmt::Debug for HostImports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostImports").finish_non_exhaustive()
    }
}

/// Error registering host imports.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub struct HostImportError;

/// Register all `host` interface imports into a linker.
///
/// Wires the generated `Host` trait methods to `StoreState`. Phase 2 stubs
/// this; Phase 3 implements the `Host` trait on `StoreState` with the real
/// callbacks and calls `add_to_linker` here.
///
/// # Errors
///
/// Returns an error if the generated `add_to_linker` rejects the wiring.
#[allow(clippy::missing_errors_doc)]
pub fn register(
    _linker: &mut Linker<crate::store::StoreState>,
    _imports: &HostImports,
) -> Result<(), error_stack::Report<HostImportError>> {
    // TODO(Phase 3): crate::bindings::jinn::plugin::host::add_to_linker(...)
    // once the Host trait is implemented on StoreState (carrying the callbacks).
    Ok(())
}
