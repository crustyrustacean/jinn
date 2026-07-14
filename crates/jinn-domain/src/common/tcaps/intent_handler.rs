//! IntentHandler capsule: the cap that gates God-mode `State::write`.
//!
//! The IntentHandler is the single-threaded platform-layer writer (synchronous
//! on the main thread, ~131 leaf handlers). It is the deliberate special-case
//! owner that keeps God-mode access. No concurrent actor holds this cap.
//!
//! `State::write()` requires `&IntentHandlerCap`. See `mint.rs` for the single
//! construction site.

/// Proof of authority to reach God-mode `State::write`. Minted only via
/// [`crate::common::tcaps::mint`]. Held by the platform layer (TUI app,
/// headless runner, discord bridge), never by a concurrent actor.
#[derive(Clone, Copy, Debug)]
pub struct IntentHandlerCap(());

impl IntentHandlerCap {
    /// Private constructor scoped to the `tcaps/` subtree.
    ///
    /// MUST be `pub(in crate::common::tcaps)`, NOT `pub(crate)`.
    pub(in crate::common::tcaps) fn new() -> Self {
        Self(())
    }
}
