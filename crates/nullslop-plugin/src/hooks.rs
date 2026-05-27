//! Hook name constants — the single source of truth for all extension points.
//!
//! Every event or command name that plugins can subscribe to or emit is
//! defined here as a `pub const`. Grepping this file shows every extension
//! point in the system.
//!
//! Hook names use the `namespace::action` convention (e.g., `"app::started"`).
