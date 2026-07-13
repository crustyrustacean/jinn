//! Compile-fail tests for the TCaps (Token-based Capability System) invariants.
//!
//! These tests assert that the compiler *rejects* code that would break the
//! capability ownership model: forging a cap, reaching private Ops fields,
//! passing a wrong-type cap, or reaching a struct absent from a facade.
//!
//! Each fixture under `tests/ui/` is a `.rs` file that must fail to compile,
//! paired with a `.stderr` capturing the expected compiler output.

#[test]
fn tcaps_compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
