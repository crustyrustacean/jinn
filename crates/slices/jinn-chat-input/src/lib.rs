//! The chat input box slice: the box's element, intent handlers, validation,
//! autocomplete rendering, and the directory-lister actor behind the
//! `@path` popup.
//!
//! The slice owns the box's keybinds as route rows and a printable-character
//! catch-all. The kernel contributes no chat-input keybind, no
//! `KernelIntent` variant, and no handler arm: everything the box does is
//! reached through the rows this slice registers.

pub mod autocomplete;
pub mod autocomplete_render;
pub mod directory_lister_actor;
pub mod element;
pub mod intent;
pub mod key_hook;
pub mod routes;
pub(crate) mod token;
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
    // The `@path` popup's own cell is not minted here: the shared cell
    // catalog (`jinn_cell_catalog::register_all_cells`) registers it in one
    // place, covering the production boot path and every test harness alike.
    let lister_deps = directory_lister_actor::DirectoryListerActorDeps {
        deps,
        state: state.clone(),
    };
    directory_lister_actor::DirectoryListerActor::spawn(host.system(), lister_deps);
    routes::attach_all(host.key_routes());

    render_regions::register(host.slices());
}

/// The chat input box's own screen region registration, split out of
/// `activate` so a composition path that cannot run activation (the TUI
/// test app) can still register the draw function. Activation claims the
/// routes and spawns the actor; this only claims the region.
pub mod render_regions {
    use jinn_kernel::common::app_state::AppState;
    use jinn_slices::DrawContext;
    use jinn_slices::DrawTarget;
    use jinn_slices::Region;

    use crate::element;

    /// Registers the chat input region — the box, and the autocomplete
    /// popup anchored to it — against `slices`.
    ///
    /// The element is stateless, so the draw function needs no interior
    /// mutability.
    pub fn register(slices: &jinn_slices::Slices) {
        slices.register_render_slot::<AppState>(
            Region::ChatInput,
            std::sync::Arc::new(
                |frame: &mut ratatui::Frame<'_>,
                 target: DrawTarget,
                 ctx: &dyn DrawContext<AppState>,
                 _rects: &mut Vec<ratatui::layout::Rect>| {
                    element::paint(frame, target.area, ctx);
                    // The autocomplete popup is drawn by the same slice, so
                    // it is part of this region rather than a second draw
                    // the composition layer has to know to issue.
                    crate::autocomplete::paint_autocomplete(frame, target.area, ctx);
                },
            ),
        );
    }
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
