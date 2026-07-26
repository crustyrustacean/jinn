//! Test-only constructors for `#[non_exhaustive]` rmcp model types.
//!
//! rmcp marks its model structs `#[non_exhaustive]`, which means they cannot be
//! constructed with a struct literal from outside the crate — even in tests.
//! These helpers provide the constructors tests need (a simple `CallToolResult`
//! with content blocks) so jinn-domain tests can exercise `format_result_content`
//! and related logic without depending on rmcp internals.

#![allow(
    clippy::missing_docs_in_private_items,
    reason = "test-only helpers for rmcp fixtures"
)]

use rmcp::model::{CallToolResult, ContentBlock};

/// Builds a [`CallToolResult`] carrying the given content blocks and
/// `is_error = false`.
#[must_use]
pub fn ok_result(content: Vec<ContentBlock>) -> CallToolResult {
    CallToolResult::success(content)
}

/// Builds a [`CallToolResult`] carrying the given content blocks and
/// `is_error = true`.
#[must_use]
pub fn error_result(content: Vec<ContentBlock>) -> CallToolResult {
    CallToolResult::error(content)
}
