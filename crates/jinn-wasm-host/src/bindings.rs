//! Generated host bindings — `wasmtime::component::bindgen!` over `wit/jinn.wit`.
//!
//! The macro reads the WIT at compile time and emits:
//!
//! - `jinn::plugin::host` — a `Host` trait (methods = the imports a plugin
//!   calls) + an `add_to_linker` helper.
//! - `jinn::plugin::hooks` — a `Hooks` trait (methods = the exports a plugin
//!   provides) + instantiate plumbing.
//! - `jinn::plugin::types` — the shared records/variants as Rust types.
//!
//! Host crate code refers to these as `crate::bindings::jinn::plugin::*`.

wasmtime::component::bindgen!({
    path: "../../wit/jinn.wit",
    world: "plugin",
    imports: { default: async | trappable },
});

/// Re-export the shared types so host code can write `crate::command::Command`
/// rather than the long generated path.
pub mod command {
    /// The typed `command` variant — the 9-verb surface.
    pub use super::jinn::plugin::types::Command;
    pub use super::jinn::plugin::types::{
        BadgeCtx, BadgeDirective, BadgeSegment, CreateSessionReq, CreateSessionResp,
        DisablePluginCmd, EnablePluginCmd, EnqueueUserMessageCmd, FireAsyncHookCmd,
        InterceptOutcome, KeybindResult, KeybindTriggerCtx, LlmOneshotReq, LlmResp,
        PushChatEntryCmd, PushEntryKind, RequestError, ResetSessionCmd, SessionCtx,
        SetChatInputCmd, SetChatInputEnabledCmd, SetManagedSessionCmd, SubmitInterceptCtx,
        TaskListCtx, ToolDecl, TurnEndCtx,
    };
}
