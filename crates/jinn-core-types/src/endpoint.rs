//! OpenRouter routing endpoint selection.

use serde::{Deserialize, Serialize};

/// A pinned OpenRouter routing endpoint.
///
/// `tag` is the OpenRouter routing slug sent as `provider.order[0]`.
/// `provider_name` is human-readable display metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    /// The OpenRouter routing slug (for example, `anthropic` or `azure`).
    pub tag: String,
    /// Human-readable upstream name (for example, `Anthropic`). Display only.
    pub provider_name: String,
}
