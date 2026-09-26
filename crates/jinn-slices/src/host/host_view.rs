//! View registration on the host: the view-facing alias, so slice
//! crates need only one import path for the registry types.

pub use crate::view::ViewSlotError as HostViewError;

/// Alias retained for the host surface: a registered view is any
/// [`crate::view::SliceView`]; the pairing check happens in
/// [`crate::host::SliceHost::viewport`].
pub type HostView<V> = V;
