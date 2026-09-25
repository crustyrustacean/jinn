//! Test-only helpers for building a configuration layer from a fixture
//! document.

/// A layer seeded with `document`, for a test that needs real config
/// rather than the shared empty layer.
///
/// Backed by an in-memory backend, so a test never touches a real
/// `~/.config`.
///
/// # Panics
///
/// Panics if `document` is not valid TOML. A malformed fixture must fail
/// loudly rather than yield a silent default that makes the assertion
/// pass for the wrong reason.
#[must_use]
#[cfg(any(test, feature = "test-harness"))]
#[expect(
    clippy::expect_used,
    reason = "a malformed test fixture must fail loudly, not yield a silent default"
)]
pub fn config_layer(document: &str) -> jinn_config::ConfigLayer {
    let parsed = document.parse().expect("test TOML parses");
    jinn_config::ConfigLayer::load(std::sync::Arc::new(
        jinn_config::InMemoryConfigStorage::new(parsed),
    ))
    .expect("layer loads")
}
