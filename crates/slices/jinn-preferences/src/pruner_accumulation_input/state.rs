//! State for the pruner accumulation threshold input popup.
//!
//! The state and its slot key live in the kernel-free
//! `jinn-preferences-msg` crate so the shared cell catalog can register
//! the slot without depending on the slice, which depends on the kernel.

pub use jinn_preferences_msg::PrunerAccumulationInputState;
