//! The chat input box slice: the box's element, intent handlers, validation,
//! autocomplete rendering, and the directory-lister actor behind the
//! `@path` popup.
//!
//! The slice owns the box's keybinds as route rows and a printable-character
//! catch-all. The kernel contributes no chat-input keybind, no
//! `KernelIntent` variant, and no handler arm: everything the box does is
//! reached through the rows this slice registers.

pub mod autocomplete_render;
pub mod directory_lister_actor;
pub mod element;
pub mod intent;
pub mod key_hook;
pub mod routes;
pub mod validator;

/// The box's state vocabulary, re-exported so slice consumers (and the
/// kernel, which must not depend on this crate) share one type.
pub use jinn_chat_input_msg::AutocompleteMatch;
pub use jinn_chat_input_msg::AutocompleteTrigger;
pub use jinn_chat_input_msg::ChatInputBoxState;
pub use jinn_chat_input_msg::InputMode;

use jinn_kernel::common::actor_deps::ActorDeps;
use jinn_kernel::common::state::State;
use jinn_kernel::common::ui_registry::UiRegistry;
use jinn_slices::SliceHost;

/// Activates the slice: the box's route rows, its key hook, and the
/// directory-lister actor behind the `@path` popup.
///
/// The `deps` and `state` are taken so the lister actor — which needs the
/// bus to receive `ListDirectory` commands and the shared state to write
/// the picker cell — can be spawned from inside the slice rather than by
/// the kernel's wiring.
pub fn activate(
    host: &mut SliceHost<'_, jinn_slices::RenderFacts>,
    deps: ActorDeps,
    state: &State,
) {
    let lister_deps = directory_lister_actor::DirectoryListerActorDeps {
        deps,
        state: state.clone(),
    };
    directory_lister_actor::DirectoryListerActor::spawn(host.system(), lister_deps);
    routes::attach_all(host.key_routes());
}

/// Registers the chat input box's element into the UI registry.
///
/// Composition calls this on every launch path: the kernel's
/// `register_all_ui_elements` cannot reference slice crates. The box is
/// fetched with `if let Some(..)`, so a missing call fails silently — the
/// box simply never draws.
pub fn register(registry: &mut UiRegistry) {
    registry.register(Box::new(element::ChatInputBoxElement));
}
