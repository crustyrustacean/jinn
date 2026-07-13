// TC1: A cap cannot be forged (constructed) from outside the tcaps/ subtree.
//
// `ProviderCap::new()` is scoped `pub(in crate::common::tcaps)`. From an
// external module (this test crate) it must be a compile error (E0624).

use jinn_domain::common::tcaps::provider::ProviderCap;

fn main() {
    let _forged = ProviderCap::new();
}
