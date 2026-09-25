//! Picker dispatch: intent handling, validation, the `AppState` host lens, and
//! geometry.
//!
//! The twelve feature *specs* live in `jinn_picker_specs` so each can follow
//! its owning slice; this module keeps the generic dispatch layer that bridges
//! kernel intents to those specs. The split is forced by the dependency
//! direction: specs read `AppState` through the host lens below, so they
//! depend on the kernel, and the kernel's `IntentHandler` dispatches into the
//! same lens — the two cannot be one crate without a cycle.
//!
//! Kernel-side entry writers wrap their items through
//! `jinn_picker::make_items_with_hooks`, never through a spec registry, so no
//! code in this module depends on `jinn_picker_specs`.

pub mod action;
pub mod geometry;
pub mod host_impl;
pub mod intent;
pub mod validator;

#[cfg(test)]
mod test_registry;
