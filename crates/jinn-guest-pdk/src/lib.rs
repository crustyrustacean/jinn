#![allow(clippy::crate_in_macro_def)]

//! jinn-guest-pdk — authoring toolkit for jinn WASM plugins.
//
// `crate::jinn::plugin::*` paths appear in macro bodies that expand in the
// *plugin* crate (where `wit_bindgen::generate!` ran). Clippy flags these as
// "crate references the macro call's crate", but the intent is correct.
//!
//! A jinn plugin is a WASM component targeting the `plugin` world (importing
//! `host`, exporting `hooks`). Because `wit-bindgen`'s generated `Guest` trait
//! and export macro are `pub(crate)` and must expand in the **final component
//! crate** (the plugin), this crate does NOT run `generate!` itself. The plugin
//! crate runs `generate!` then calls [`plugin!`], which injects the ergonomic
//! helper modules (`host`, `bag`, `manifest`) and a `prelude` into the plugin
//! crate and wires the component export.
//!
//! Minimal plugin (see `plugins/welcome/`):
//!
//! ```ignore
//! wit_bindgen::generate!({ path: "../../wit/jinn.wit", world: "plugin" });
//! jinn_guest_pdk::plugin!(Welcome);
//! ```

pub use postcard;
pub use serde;

#[macro_use]
mod modules;
mod macros;
