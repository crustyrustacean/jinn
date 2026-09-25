//! Transitional frontend cap retained until cap-plumbing migration completes.

/// Proof of authority to write frontend state.
#[derive(Clone, Copy, Debug)]
pub struct FrontendCap(());

impl FrontendCap {
    pub(in crate::common::tcaps) fn new() -> Self {
        Self(())
    }
}
