//! A naming seam between tool call arguments and the provider registry.
//!
//! Tool arguments that name a model arrive as a bare string from the model
//! on the wire. Deciding whether that string names something real — an
//! expanded `provider/model` id, or a configured alias — is registry
//! knowledge, and the tools layer must not depend on the provider config
//! crate to get it. This module defines the narrow question the tools layer
//! is allowed to ask, as a trait over plain strings; the kernel provides the
//! implementation backed by [`jinn_provider_config::ProviderRegistryService`].
//!
//! The trait lives here — beside [`crate::ModelSelection`], whose `Single`
//! payload is the string a resolution produces — rather than in a
//! provider-specific crate, so that a tool crate can carry the seam across
//! its boundary without adding any provider dependency.

/// Resolves a model name from a tool argument to a canonical model id.
///
/// Two spellings resolve: the expanded `provider/model` id (which resolves
/// to itself), and a configured alias name (which resolves to its target
/// expanded id). A name that is both resolves as the expanded id — registry
/// lookup precedes alias resolution. Anything else is `None`, and the
/// caller decides what an unknown name means for it.
///
/// Implemented by the kernel over the live provider registry; see
/// `jinn_kernel::common::provider_resolution`.
pub trait ModelNameResolver: Send + Sync {
    /// Resolves `name` to a canonical expanded `provider/model` id, or
    /// `None` when the name is neither an expanded id nor a configured
    /// alias.
    fn resolve(&self, name: &str) -> Option<String>;

    /// The resolver's debug name (service-trait convention).
    fn name(&self) -> &'static str {
        "model-name-resolver"
    }
}
