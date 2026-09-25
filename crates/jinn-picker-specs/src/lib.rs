//! Picker specs and the `AppState` host lens — the kernel's picker adapter.
//!
//! The twelve feature specs live here rather than in the kernel so each can
//! later follow its owning slice. They are the composition side of
//! [`jinn_picker`]: each spec authors one picker's row rendering, search,
//! preview, keybinds, and lifecycle in one place, and `build_picker_registry`
//! registers them all.
//!
//! # Why this crate depends on the kernel
//!
//! Every spec's confirm and open hooks read and write [`AppState`] through
//! the host's `state_any` bridge. That bridge is **temporary**: it downcasts
//! the whole application state to reach a few cells, so a spec depends on the
//! kernel in its entirety rather than only on what it uses. The debt is paid
//! down spec by spec, each one moving to its owning slice as that family
//! graduates to cells:
//!
//! | Spec | Owning slice |
//! |------|--------------|
//! | `persona` | `jinn-persona` |
//! | `skill` | the skills family |
//! | `theme` | `jinn-theme-slice` |
//! | `tool` | `jinn-tools` |
//! | `mcp_server` | `jinn-mcp-slice` |
//! | `session_lifecycle` | `jinn-session-init` |
//! | `task_list` | the task family |
//! | `session` | the session family |
//! | `provider` | `jinn-provider-selection` |
//! | `endpoint` | `jinn-provider-selection` |
//! | `reasoning_effort` | `jinn-provider-selection` |
//! | `project` | the project family |
//!
//! # The cycle guard
//!
//! **The kernel must never import this crate.** These specs import
//! `jinn_domain`; if `jinn_domain` also imported them, the dependency graph
//! would contain a cycle. The kernel's `IntentHandler`, its `AppState` host
//! lens, and the entry types they store all stay kernel-side for the same
//! reason — the dispatch layer depends on the host, so the specs depend on the
//! dispatch layer's crate. Kernel code that needs picker ids or the framework
//! uses `jinn_picker` directly, and kernel entry writers wrap their items
//! through `jinn_picker::make_items_with_hooks` rather than consulting a spec
//! registry. Composition (the TUI and the slice crates) imports this crate to
//! obtain the registered specs.

pub mod endpoint_spec;
pub mod mcp_server_spec;
pub mod persona_spec;
pub mod project_spec;
pub mod provider_spec;
pub mod reasoning_effort_spec;
pub mod session_lifecycle_spec;
pub mod session_spec;
pub mod skill_spec;
pub mod task_list_spec;
pub mod theme_spec;
pub mod tool_spec;

mod registry;

pub use registry::build_picker_registry;
