//! Transitional session cap retained until the cap-plumbing migration completes.

/// Proof of authority to write the session collection.
#[derive(Clone, Copy, Debug)]
pub struct SessionCap(());

impl SessionCap {
    pub(in crate::common::tcaps) fn new() -> Self {
        Self(())
    }
}
