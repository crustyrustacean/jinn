//! Overlay view registry — re-export of the shared slice registry.
//!
//! The kernel previously owned a `RenderCtx`-taking registry; slices
//! now register [`RenderFacts`]-taking renderers through the host, and
//! this module re-exports the shared type so existing call sites
//! compile unchanged.

pub use jinn_slices::overlay::OverlayViewFn;
pub use jinn_slices::overlay::OverlayViews;
