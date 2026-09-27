//! Composition: the single place the boot wiring lives.
//!
//! - [`slices`] — the boot list: every slice activation, in order.
//! - [`registries`] — the one borrow an activation gets.
//! - [`ctx`] — the inputs an activation needs beyond its host.
//! - [`ui`] — the slice-owned UI element registrations.
//!
//! To answer "what runs at launch, and in what order", read `slices.rs`.

pub mod config;
pub mod ctx;

pub mod slices;
pub mod ui;

pub use ctx::Ctx;

pub use slices::ActivateError;
