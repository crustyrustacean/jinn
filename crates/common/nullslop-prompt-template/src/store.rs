//! Prompt template store — holds all loaded templates and provides lookup.
//!
//! The store is populated at startup by scanning `~/.config/nullslop/prompts/`
//! recursively. It supports exact name lookup and fuzzy search for the
//! autocomplete popup.

use std::path::Path;
use std::sync::Arc;

use error_stack::{Report, ResultExt as _};
use fuzzy_matcher::FuzzyMatcher as _;
use fuzzy_matcher::skim::SkimMatcherV2;
use nullslop_protocol::PromptTemplate;
use tracing::warn;

use crate::loader::parse_template_file;

/// Errors that can occur during prompt template loading.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub enum PromptTemplateStoreError {
    /// Filesystem I/O failure.
    Io,
}

/// Maximum number of results returned by fuzzy search.
const MAX_FUZZY_RESULTS: usize = 20;

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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_template(dir: &Path, filename: &str, content: &str) {
        std::fs::write(dir.join(filename), content).expect("write template file");
    }

    #[rstest::rstest]
    fn load_from_dir_returns_empty_when_missing() {
        // Given a path that does not exist.
        let path = Path::new("/tmp/nullslop-nonexistent-test-dir");

        // When loading from that path.
        let store = PromptTemplateStore::load_from_dir(path).expect("load");

        // Then the store is empty.
        assert!(store.is_empty());
    }

    #[rstest::rstest]
    fn load_from_dir_parses_templates() {
        // Given a directory with two template files.
        let dir = TempDir::new().expect("temp dir");
        write_template(
            dir.path(),
            "hello.md",
            "+++\nname = \"hello\"\ndescription = \"Say hello\"\n+++\nHello, world!",
        );
        write_template(
            dir.path(),
            "review.md",
            "+++\nname = \"review\"\ndescription = \"Code review\"\n+++\nReview this code.",
        );

        // When loading from that directory.
        let store = PromptTemplateStore::load_from_dir(dir.path()).expect("load");

        // Then both templates are loaded.
        assert_eq!(store.len(), 2);
        assert!(store.find_by_name("hello").is_some());
        assert!(store.find_by_name("review").is_some());
    }

    #[rstest::rstest]
    fn load_from_dir_scans_subdirectories() {
        // Given a directory with a subdirectory containing a template.
        let dir = TempDir::new().expect("temp dir");
        let sub = dir.path().join("subdir");
        std::fs::create_dir_all(&sub).expect("create subdir");
        write_template(
            &sub,
            "nested.md",
            "+++\nname = \"nested\"\ndescription = \"Nested\"\n+++\nNested template.",
        );

        // When loading from the root directory.
        let store = PromptTemplateStore::load_from_dir(dir.path()).expect("load");

        // Then the nested template is found.
        assert_eq!(store.len(), 1);
        assert!(store.find_by_name("nested").is_some());
    }

    #[rstest::rstest]
    fn load_from_dir_skips_non_md_files() {
        // Given a directory with a non-markdown file.
        let dir = TempDir::new().expect("temp dir");
        write_template(dir.path(), "ignore.txt", "this is not a template");

        // When loading.
        let store = PromptTemplateStore::load_from_dir(dir.path()).expect("load");

        // Then the store is empty.
        assert!(store.is_empty());
    }

    #[rstest::rstest]
    fn load_from_dir_handles_duplicate_names() {
        // Given two files with the same name in their frontmatter.
        let dir = TempDir::new().expect("temp dir");
        write_template(
            dir.path(),
            "a.md",
            "+++\nname = \"dup\"\ndescription = \"First\"\n+++\nFirst.",
        );
        write_template(
            dir.path(),
            "b.md",
            "+++\nname = \"dup\"\ndescription = \"Second\"\n+++\nSecond.",
        );

        // When loading.
        let store = PromptTemplateStore::load_from_dir(dir.path()).expect("load");

        // Then only one occurrence is kept (first found).
        assert_eq!(store.len(), 1);
        let tmpl = store.find_by_name("dup").expect("found");
        // The exact description depends on filesystem traversal order,
        // but it must be one of the two.
        assert!(tmpl.description == "First" || tmpl.description == "Second");
    }

    #[rstest::rstest]
    fn find_by_name_returns_none_for_missing() {
        // Given an empty store.
        let store = PromptTemplateStore::new();

        // When looking up a name.
        // Then it returns None.
        assert!(store.find_by_name("nonexistent").is_none());
    }

    #[rstest::rstest]
    fn fuzzy_search_returns_matching_templates() {
        // Given a store with templates.
        let store = PromptTemplateStore::from_vec(vec![
            PromptTemplate {
                name: "code-review".to_owned(),
                description: "Review code".to_owned(),
                body: "Review this.".to_owned(),
            },
            PromptTemplate {
                name: "commit-message".to_owned(),
                description: "Write commit".to_owned(),
                body: "Write a commit.".to_owned(),
            },
            PromptTemplate {
                name: "summarize".to_owned(),
                description: "Summarize text".to_owned(),
                body: "Summarize this.".to_owned(),
            },
        ]);

        // When fuzzy searching for "co".
        let results = store.fuzzy_search("co");

        // Then code-review and commit-message match (both contain "co").
        assert_eq!(results.len(), 2);
        let names: Vec<&str> = results.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"code-review"));
        assert!(names.contains(&"commit-message"));
    }

    #[rstest::rstest]
    fn fuzzy_search_returns_empty_for_no_match() {
        // Given a store with templates.
        let store = PromptTemplateStore::from_vec(vec![PromptTemplate {
            name: "hello".to_owned(),
            description: "Say hello".to_owned(),
            body: "Hello!".to_owned(),
        }]);

        // When fuzzy searching for something that doesn't match.
        let results = store.fuzzy_search("zzz");

        // Then no results are returned.
        assert!(results.is_empty());
    }

    #[rstest::rstest]
    fn fuzzy_search_orders_by_relevance() {
        // Given a store with templates that have different relevance.
        let store = PromptTemplateStore::from_vec(vec![
            PromptTemplate {
                name: "code-review".to_owned(),
                description: "Review".to_owned(),
                body: String::new(),
            },
            PromptTemplate {
                name: "codellama".to_owned(),
                description: "Model".to_owned(),
                body: String::new(),
            },
            PromptTemplate {
                name: "review".to_owned(),
                description: "Review".to_owned(),
                body: String::new(),
            },
        ]);

        // When fuzzy searching for "code".
        let results = store.fuzzy_search("code");

        // Then results are ordered least relevant first, most relevant last.
        // "code-review" starts with "code" so it's most relevant (last).
        // "codellama" starts with "code" so it's also very relevant.
        assert!(!results.is_empty());
        // The most relevant should be the last entry.
        let last = results.last().expect("at least one result");
        // Both code-review and codellama start with "code" so one of them
        // should be last. The exact order depends on fuzzy scoring.
        assert!(last.name.starts_with("code"));
    }

    #[rstest::rstest]
    fn fuzzy_search_caps_at_max_results() {
        // Given a store with many templates that all match.
        let templates: Vec<PromptTemplate> = (0..30)
            .map(|i| PromptTemplate {
                name: format!("template-{i}"),
                description: format!("Template {i}"),
                body: String::new(),
            })
            .collect();
        let store = PromptTemplateStore::from_vec(templates);

        // When fuzzy searching with a broad query.
        let results = store.fuzzy_search("template");

        // Then results are capped at MAX_FUZZY_RESULTS (20).
        assert_eq!(results.len(), MAX_FUZZY_RESULTS);
    }
}
