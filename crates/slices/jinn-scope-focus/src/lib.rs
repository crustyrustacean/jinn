//! The scope-focus slice — the sync interaction substrate.
//!
//! Owns one cell ([`scope_focus_slot`]) bundling the focus-scope
//! stack, the TUI signals, and the quit latch. The kernel's
//! IntentHandler (and the intent fns it dispatches to) writes through
//! a facade on `FrontendState`; the TUI render pass, keymap
//! generation, and the run loop read it. There is no actor and no
//! route row: the only writer is the exempt sync IntentHandler, and
//! the state is not a rendered element.

pub use jinn_slices::ScopeFocusState;
pub use jinn_slices::scope_focus_slot;

use jinn_slices::SliceHost;

/// Activates the slice. No routes, no actors, no view.
///
/// The scope-focus cell is not minted here: the shared cell catalog
/// (`jinn_cell_catalog::register_all_cells`) registers every slice cell in
/// one place, before any slice activates.
///
/// # Panics
///
/// Never panics: nothing is registered, so there is nothing to assert.
pub fn activate(_host: &mut SliceHost<'_, jinn_slices::RenderFacts>) {}
