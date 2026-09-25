//! Compatibility exports for portable `+++` TOML frontmatter parsing.
//!
//! The implementation is owned by `jinn-context`; this module preserves the
//! existing kernel path until all callers are migrated.

pub use jinn_context::frontmatter::{FrontmatterError, parse_toml_frontmatter};
