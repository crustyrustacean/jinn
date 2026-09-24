// TC1: A cap cannot be forged (constructed) from outside the tcaps/ subtree.
//
// `SessionCap::new()` is scoped `pub(in crate::common::tcaps)`. From an
// external module (this test crate) it must be a compile error (E0624).
//
// (The provider capsule was dissolved in the provider-selection window; the
// invariant is re-anchored on the session capsule.)

use jinn_domain::common::tcaps::SessionCap;

fn main() {
    let _forged = SessionCap::new();
}
