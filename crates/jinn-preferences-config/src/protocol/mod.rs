//! App-state bus protocol — commands.
//!
//! [`app_state_command::UpdateAppState`] crosses slice boundaries (the
//! sidebar emits it), so it lives beside the schema in the kernel-free
//! config crate with its [`BusMessage`] impl. The typed `MsgHandler` impl
//! that delivers it lives with the actor in the `jinn-preferences` slice
//! (orphan rule: the actor is local there).
//!
//! `jinn.toml` has no command here. The configuration layer writes it
//! directly, so there is no bus path to route.

pub mod app_state_command;
