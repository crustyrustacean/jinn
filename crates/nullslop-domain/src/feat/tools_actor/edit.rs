//! Edit built-in tool — performs exact text replacement in files.
//!
//! Supports multiple non-overlapping edits per call, BOM preservation,
//! line-ending preservation, and unified diff output.

mod diff;
mod line_ending;

use std::path::{Path, PathBuf};

use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};

use super::BoxedToolFuture;
use diff::Edit;

// pub use diff::generate_unified_diff; // Not needed outside the module yet
/// Returns the tool definition for the `edit` built-in tool.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "edit".to_owned(),
        description: "Edit a file using exact text replacement. \
            Each oldText must match a unique, non-overlapping region of the original file. \
            Returns a unified diff of the changes."
            .to_owned(),
        prompt_snippet: Some(
            "Make precise file edits with exact text replacement, including multiple disjoint edits in one call"
                .to_owned(),
        ),
        prompt_guidelines: vec![
            "Use edit for precise changes (edits[].oldText must match exactly).".to_owned(),
            "When changing multiple separate locations in one file, use one edit call with multiple entries in edits[] instead of multiple edit calls.".to_owned(),
            "Each edits[].oldText is matched against the original file, not incrementally. Do not emit overlapping or nested edits. Merge nearby changes into one edit.".to_owned(),
            "Keep edits[].oldText as small as possible while still being unique in the file. Do not pad with large unchanged regions.".to_owned(),
        ],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit (relative or absolute)"
                },
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "oldText": {
                                "type": "string",
                                "description": "Exact text to find in the file (must be unique)"
                            },
                            "newText": {
                                "type": "string",
                                "description": "Replacement text"
                            }
                        },
                        "required": ["oldText", "newText"],
                        "additionalProperties": false
                    },
                    "description": "One or more text replacements to apply"
                }
            },
            "required": ["path", "edits"],
            "additionalProperties": false
        }),
    }
}

/// Resolves a path against the CWD if relative, returns absolute as-is.
fn resolve_path(path: &str, cwd: &Path) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_owned()
    } else {
        cwd.join(p)
    }
}

/// Executes the `edit` built-in tool.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let (path, edits) = match parse_args(&call.arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: format!("failed to parse arguments: {e}"),
                    success: false,
                    full_content: None,
                    truncation: None,
                };
            }
        };

        if edits.is_empty() {
            return ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: "no edits provided".to_owned(),
                success: false,
                full_content: None,
                truncation: None,
            };
        }

        let resolved = resolve_path(&path, &ctx.cwd);

        // Read file
        let raw_content = match tokio::fs::read_to_string(&resolved).await {
            Ok(c) => c,
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: format!("failed to read file '{}': {e}", resolved.display()),
                    success: false,
                    full_content: None,
                    truncation: None,
                };
            }
        };

        // Strip BOM and detect line endings
        let (content, bom) = line_ending::strip_bom(&raw_content);
        let detected_ending = line_ending::detect_line_ending(content);

        // Normalize to LF for matching
        let normalized = line_ending::normalize_to_lf(content);

        // Find and validate edits against normalized content
        let matched = match diff::find_and_validate_edits(&normalized, &edits) {
            Ok(m) => m,
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: format!("edit validation failed: {e}"),
                    success: false,
                    full_content: None,
                    truncation: None,
                };
            }
        };

        // Apply edits
        let modified_normalized = diff::apply_edits(&normalized, &matched);

        // Restore line endings
        let modified = line_ending::restore_line_endings(&modified_normalized, detected_ending);

        // Re-add BOM if present
        let final_content = match bom {
            Some(b) => format!("{b}{modified}"),
            None => modified,
        };

        // Generate diff against normalized content for consistent output
        let diff = diff::generate_unified_diff(&normalized, &modified_normalized, &path);

        // Write back
        if let Err(e) = tokio::fs::write(&resolved, &final_content).await {
            return ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: format!("failed to write file '{}': {e}", resolved.display()),
                success: false,
                full_content: None,
                truncation: None,
            };
        }

        ToolResult {
            tool_call_id: call.id,
            name: call.name,
            content: format!(
                "Successfully replaced {} block(s) in {}.\n{}",
                matched.len(),
                resolved.display(),
                diff
            ),
            success: true,
            full_content: None,
            truncation: None,
        }
    })
}

/// Parses the arguments from the tool call JSON.
fn parse_args(raw: &str) -> Result<(String, Vec<Edit>), serde_json::Error> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    let path = v
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();

    let edits = v
        .get("edits")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let old_text = item.get("oldText")?.as_str()?.to_owned();
                    let new_text = item.get("newText")?.as_str()?.to_owned();
                    Some(Edit { old_text, new_text })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok((path, edits))
}
