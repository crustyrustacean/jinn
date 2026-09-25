//! Compatibility exports for portable prompt and attachment context helpers.
//!
//! Values, parsing, lookup, expansion, and attachment scanning are owned by
//! `jinn-context`. This kernel path remains until consumers are migrated.

pub use jinn_context::attachment_path::{
    ImageKind, PathResolveContext, PendingPath, ScanResult, classify_image_bytes, scan_at_paths,
    scan_at_paths_with_degraded,
};
pub use jinn_context::{
    PromptTemplateParseError, PromptTemplateStore, PromptTemplateStoreError, expand_tokens,
    render_template_file,
};
