//! Session-init crossing contracts — the prompt-template scan pair and
//! the [`PromptTemplate`] noun.
//!
//! Re-homed from the kernel `feat/provider/protocol/` (the pair) and
//! `feat/context/protocol/prompt_template.rs` (the noun) in the
//! provider-selection window: the pair's producer is this slice's prompt
//! scan worker, so its contracts live where the producer lives (the
//! `SkillsLoaded`/`ScanSkills` precedent). The crossing-schema ids
//! ("RescanPromptTemplates", "PromptTemplatesLoaded") are unchanged.
//!
//! `PromptTemplateStore` and the loading/parsing stay kernel-side until
//! the context family migrates; this crate carries only the wire noun so
//! it has no kernel dependency.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use jinn_core_types::SessionId;
use jinn_slices::BusMessage;

/// A reusable prompt template loaded from `~/.config/jinn/prompts/`.
///
/// Parsed from a markdown file with TOML frontmatter:
///
/// ```markdown
/// +++
/// name = "code-review"
/// description = "Perform a thorough code review"
/// +++
/// You are an expert code reviewer...
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplate {
    /// Unique identifier used in `$name` references.
    pub name: String,
    /// Short human-readable description shown in the autocomplete popup.
    pub description: String,
    /// The full template body text.
    pub body: String,
}

/// Rescan prompt templates for a specific session.
///
/// Carries the session's cwd: the worker scans user/system plus project-local
/// `.agents/prompts` dirs (most-local wins), and emits [`PromptTemplatesLoaded`].
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Rescan prompt templates for a session.")]
pub struct RescanPromptTemplates {
    /// The session whose scan this is.
    pub session_id: SessionId,
    /// The working directory driving the scan.
    #[serde(default)]
    pub cwd: PathBuf,
}
impl BusMessage for RescanPromptTemplates {}

/// Prompt templates loaded after a rescan.
///
/// Emitted by the prompt scan actor after scanning the prompts directory.
/// On success, `templates` contains the loaded templates and `error` is `None`.
/// On failure, `templates` is empty and `error` contains a description.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "Prompt templates loaded after a rescan.")]
pub struct PromptTemplatesLoaded {
    /// The session whose cwd drove the scan.
    pub session_id: SessionId,
    /// The loaded prompt templates.
    pub templates: Vec<PromptTemplate>,
    /// Error message if scanning failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl BusMessage for PromptTemplatesLoaded {}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, reason = "test code")]

    use super::*;

    #[rstest::rstest]
    #[case("RescanPromptTemplates")]
    #[case("PromptTemplatesLoaded")]
    fn crossing_schema_ids_are_stable(#[case] name: &str) {
        // Given the two crossing contracts.
        // When deriving their trouper schema ids.
        // Then the ids equal the type names (the compatibility surface).
        let id = match name {
            "RescanPromptTemplates" => {
                <RescanPromptTemplates as trouper::schema::Schema>::schema_id()
            }
            "PromptTemplatesLoaded" => {
                <PromptTemplatesLoaded as trouper::schema::Schema>::schema_id()
            }
            other => unreachable!("unhandled name: {other}"),
        };
        // Then the id is the type name (the compatibility surface).
        assert_eq!(id.name(), name);
    }

    #[rstest::rstest]
    #[test]
    fn prompt_template_roundtrips_through_serde() {
        // Given a prompt template.
        let template = PromptTemplate {
            name: "code-review".to_owned(),
            description: "Review the diff".to_owned(),
            body: "You are an expert...".to_owned(),
        };

        // When serializing and deserializing.
        let json = serde_json::to_string(&template).expect("serialize");
        let round: PromptTemplate = serde_json::from_str(&json).expect("deserialize");

        // Then the fields survive the roundtrip.
        assert_eq!(round.name, "code-review");
        assert_eq!(round.description, "Review the diff");
        assert_eq!(round.body, "You are an expert...");
    }
}
