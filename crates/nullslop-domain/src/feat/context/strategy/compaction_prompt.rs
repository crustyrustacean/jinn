//! Compaction prompt template loading with bundled fallback.
//!
//! The compaction prompt template defines the instructions given to the LLM
//! when generating context summaries. Users can override it by placing a
//! `compaction.md` file in their config directory. If no override exists,
//! the bundled default (from `prompts/compaction.md`) is used.

const DEFAULT_COMPACTION_PROMPT: &str = include_str!("../../../../../../prompts/compaction.md");

/// Load the compaction prompt template.
///
/// Checks for a user override at `config_dir/compaction.md` first,
/// then falls back to the bundled default.
///
/// # Panics
///
/// Does not panic — always returns a valid prompt string.
pub fn load_compaction_prompt(config_dir: &std::path::Path) -> String {
    let override_path = config_dir.join("compaction.md");
    if override_path.exists() {
        std::fs::read_to_string(&override_path).unwrap_or_else(|e| {
            tracing::warn!(
                path = %override_path.display(),
                error = %e,
                "failed to read compaction prompt override, using bundled default"
            );
            DEFAULT_COMPACTION_PROMPT.to_owned()
        })
    } else {
        DEFAULT_COMPACTION_PROMPT.to_owned()
    }
}

/// Returns the bundled default compaction prompt.
///
/// Use when no config directory is available (e.g., in tests).
#[must_use]
pub fn default_compaction_prompt() -> &'static str {
    DEFAULT_COMPACTION_PROMPT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn default_prompt_contains_goal_section() {
        assert!(DEFAULT_COMPACTION_PROMPT.contains("## Goal"));
    }

    #[rstest::rstest]
    fn default_prompt_contains_progress_section() {
        assert!(DEFAULT_COMPACTION_PROMPT.contains("## Progress"));
    }

    #[rstest::rstest]
    fn default_prompt_contains_key_decisions_section() {
        assert!(DEFAULT_COMPACTION_PROMPT.contains("## Key Decisions"));
    }

    #[rstest::rstest]
    fn default_prompt_contains_critical_context_section() {
        assert!(DEFAULT_COMPACTION_PROMPT.contains("## Critical Context"));
    }

    #[rstest::rstest]
    fn load_returns_default_when_no_override() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let prompt = load_compaction_prompt(dir.path());
        assert!(prompt.contains("## Goal"));
    }

    #[rstest::rstest]
    fn load_uses_override_when_present() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("compaction.md"), "Custom override prompt").expect("write");
        let prompt = load_compaction_prompt(dir.path());
        assert_eq!(prompt, "Custom override prompt");
    }

    #[rstest::rstest]
    fn default_prompt_is_not_empty() {
        assert!(!DEFAULT_COMPACTION_PROMPT.is_empty());
    }
}
