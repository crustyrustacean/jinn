//! View registration on the host: the typed tab-view verb lives in
//! the host's `register_view`; this module carries the view-facing
//! alias so slice crates need only one import path.

pub use crate::view::ViewSlotError as HostViewError;

/// Alias retained for the host surface: a registered view is any
/// [`crate::view::SliceView`]; the pairing check happens in
/// [`crate::host::SliceHost::register_view`].
pub type HostView<V> = V;
