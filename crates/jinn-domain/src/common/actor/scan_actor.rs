//! Marker type for actors that don't accept direct messages.

/// Marker type for actors that don't use direct messages.
///
/// Use as `type Message = NoDirectMsg;` in the `Actor` impl.
pub enum NoDirectMsg {}
