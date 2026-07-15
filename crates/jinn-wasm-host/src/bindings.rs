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
});

/// Re-export the shared types so host code can write `crate::command::Command`
/// rather than the long generated path.
pub mod command {
    pub use super::jinn::plugin::types::{
        BadgeCtx, BadgeDirective, BadgeSegment, CreateSessionReq, CreateSessionResp,
        InterceptOutcome, KeybindResult, KeybindTriggerCtx, LlmOneshotReq, LlmResp,
        RequestError, SessionCtx, SubmitInterceptCtx, TaskListCtx, ToolDecl, TurnEndCtx,
    };
    /// The typed `command` variant — the 9-verb surface.
    pub use super::jinn::plugin::types::Command;
}
