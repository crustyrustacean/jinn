//! Prompt template store — holds all loaded templates and provides lookup.
//!
//! The store is populated at startup by scanning `~/.config/nullslop/prompts/`
//! recursively. It supports exact name lookup and fuzzy search for the
//! autocomplete popup.

use std::path::Path;
use std::sync::Arc;

use crate::protocol::PromptTemplate;
use error_stack::{Report, ResultExt as _};
use fuzzy_matcher::FuzzyMatcher as _;
use fuzzy_matcher::skim::SkimMatcherV2;
use tracing::warn;

use super::loader::parse_template_file;

/// Errors that can occur during prompt template loading.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub enum PromptTemplateStoreError {
    /// Filesystem I/O failure.
    Io,
}

/// Maximum number of results returned by fuzzy search.
pub const MAX_FUZZY_RESULTS: usize = 20;

/// Holds all loaded prompt templates and provides lookup methods.
///
/// Uses `Arc<[PromptTemplate]>` internally so cloning is cheap — the store
/// lives in `AppState` behind an `RwLock`.
#[derive(Debug, Clone, Default)]
pub struct PromptTemplateStore {
    /// All loaded templates.
    templates: Arc<[PromptTemplate]>,
}

impl PromptTemplateStore {
    /// Creates an empty store with no templates.
    #[must_use]
    pub fn new() -> Self {
        Self {
            templates: Arc::new([]),
        }
    }

    /// Loads all `*.md` files from the given directory (recursively) and
    /// returns a populated store.
    ///
    /// If the directory does not exist, returns an empty store (no error).
    /// Files that fail to parse are logged as warnings and skipped.
    /// Duplicate names are logged as warnings; the first occurrence wins.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be read due to I/O failure.
    pub fn load_from_dir(path: &Path) -> Result<Self, Report<PromptTemplateStoreError>> {
        if !path.exists() {
            return Ok(Self::new());
        }

        let mut templates = Vec::new();
        let mut seen_names = std::collections::HashSet::new();

        Self::scan_dir(path, &mut templates, &mut seen_names)?;

        Ok(Self {
            templates: templates.into(),
        })
    }

    /// Creates a store from a pre-built list of templates (for testing).
    #[must_use]
    pub fn from_vec(templates: Vec<PromptTemplate>) -> Self {
        Self {
            templates: templates.into(),
        }
    }

    /// Returns the template with the given name, if it exists.
    #[must_use]
    pub fn find_by_name(&self, name: &str) -> Option<&PromptTemplate> {
        self.templates.iter().find(|t| t.name == name)
    }

    /// Returns fuzzy-matched templates ordered by relevance score.
    ///
    /// Results are ordered least-relevant (top) to most-relevant (bottom),
    /// capped at \[`MAX_FUZZY_RESULTS`].
    #[must_use]
    pub fn fuzzy_search(&self, query: &str) -> Vec<&PromptTemplate> {
        let matcher = SkimMatcherV2::default();

        let mut scored: Vec<(i64, &PromptTemplate)> = self
            .templates
            .iter()
            .filter_map(|t| matcher.fuzzy_match(&t.name, query).map(|score| (score, t)))
            .collect();

        // Sort by score ascending (least relevant first, most relevant last).
        scored.sort_by_key(|(score, _)| *score);
        scored.truncate(MAX_FUZZY_RESULTS);

        scored.into_iter().map(|(_, t)| t).collect()
    }

    /// Returns the number of loaded templates.
    #[must_use]
    pub fn len(&self) -> usize {
        self.templates.len()
    }

    /// Returns `true` if there are no loaded templates.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.templates.is_empty()
    }

    /// Returns a reference to the full template list.
    #[must_use]
    pub fn templates(&self) -> &[PromptTemplate] {
        &self.templates
    }

    /// Recursively scans a directory for `*.md` files and parses them.
    fn scan_dir(
        dir: &Path,
        templates: &mut Vec<PromptTemplate>,
        seen_names: &mut std::collections::HashSet<String>,
    ) -> Result<(), Report<PromptTemplateStoreError>> {
        let entries = std::fs::read_dir(dir)
            .change_context(PromptTemplateStoreError::Io)
            .attach(format!("failed to read directory {}", dir.display()))?;

        for entry in entries {
            let entry: std::fs::DirEntry = entry
                .change_context(PromptTemplateStoreError::Io)
                .attach(format!("failed to read dir entry in {}", dir.display()))?;

            let path = entry.path();

            if path.is_dir() {
                Self::scan_dir(&path, templates, seen_names)?;
                continue;
            }

            if path.extension().is_none_or(|ext| ext != "md") {
                continue;
            }

            match parse_template_file(&path) {
                Ok(template) => {
                    if seen_names.contains(&template.name) {
                        warn!(
                            name = %template.name,
                            path = %path.display(),
                            "duplicate prompt template name, skipping"
                        );
                    } else {
                        seen_names.insert(template.name.clone());
                        templates.push(template);
                    }
                }
                Err(e) => {
                    warn!(
                        path = %path.display(),
                        error = %e,
                        "failed to parse prompt template, skipping"
                    );
                }
            }
        }

        Ok(())
    }
}
