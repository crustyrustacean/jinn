//! The slice-owned UI element registrations.
//!
//! Four slices render an element the kernel's registry cannot name,
//! because `jinn-kernel` must not depend on a slice crate. Those
//! registrations are composition's job.
//!
//! This function lives in `jinn-tui` rather than in the binary's
//! bootstrap module because the test builder needs the same list: a
//! second copy in the test path would drift, and a slice that
//! registered a new element would draw in the app and not in its own
//! tests. The composition root calls this from `bootstrap::ui`.

use jinn_kernel::AppUiRegistry;

/// Builds the UI element registry for a launch.
///
/// The kernel's own elements register first, then the four slice-owned
/// ones. The chat input box is fetched with `if let Some(..)`, so a
/// missing registration fails silently — the box simply never draws.
#[must_use]
pub fn build_ui_registry() -> AppUiRegistry {
    let mut ui_registry = AppUiRegistry::new();
    jinn_kernel::register_all_ui_elements(&mut ui_registry);
    jinn_chat_log_view::register(&mut ui_registry);
    jinn_inference::register(&mut ui_registry);
    jinn_status_bar::register(&mut ui_registry);
    jinn_chat_input::register(&mut ui_registry);
    ui_registry
}
