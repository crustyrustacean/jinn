//! Workflow definition store trait, file-backed implementation, and service wrapper.
//!
//! Defines [`WorkflowStore`] as the async trait for workflow definition persistence
//! and [`FileWorkflowStore`] as the per-file JSON backend. Each workflow is
//! stored as `<name>.json` in `~/.config/nullslop/workflows/`.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use error_stack::{Report, ResultExt as _};
use nullslop_workflow::WorkflowDef;
use wherror::Error;

/// Directory name under the platform config directory.
const DIR_NAME: &str = "nullslop";

/// Subdirectory for workflow definitions.
const WORKFLOWS_DIR: &str = "workflows";

/// Error type for workflow store operations.
#[derive(Debug, Error)]
#[error(debug)]
pub struct WorkflowStoreError;

/// Abstraction for workflow definition persistence.
///
/// Every external dependency must have a trait abstraction. Filesystem I/O
/// is an external dependency — this trait abstracts it so tests can swap
/// in-memory storage.
///
/// Methods are `async` because the primary implementation ([`FileWorkflowStore`])
/// runs inside an async actor on a tokio task and uses `tokio::fs`.
#[async_trait]
pub trait WorkflowStore: Send + Sync + 'static {
    /// Returns the storage backend name (for debugging).
    fn name(&self) -> &'static str;

    /// Save a workflow definition, creating or overwriting the file.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowStoreError`] if the write fails.
    async fn save(&self, name: &str, def: &WorkflowDef) -> Result<(), Report<WorkflowStoreError>>;

    /// Load a workflow definition by name.
    ///
    /// Returns `None` if no workflow with that name exists.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowStoreError`] if the read or parse fails.
    async fn load(&self, name: &str) -> Result<Option<WorkflowDef>, Report<WorkflowStoreError>>;

    /// List all saved workflow definitions.
    ///
    /// Returns `(name, description)` pairs for every workflow in the store.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowStoreError`] if the directory cannot be read.
    async fn list(&self) -> Result<Vec<(String, String)>, Report<WorkflowStoreError>>;

    /// Delete a workflow definition by name.
    ///
    /// Returns `Ok(())` even if the workflow did not exist.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowStoreError`] if the delete fails.
    async fn delete(&self, name: &str) -> Result<(), Report<WorkflowStoreError>>;
}

/// Service wrapper for workflow definition storage.
///
/// Wraps `Arc<dyn WorkflowStore>` for shared ownership across the application.
/// Follows the service wrapper pattern from the project style guide.
#[derive(Debug, Clone)]
pub struct WorkflowStoreService {
    /// The underlying workflow store implementation.
    svc: Arc<dyn WorkflowStore>,
}

impl WorkflowStoreService {
    /// Creates a new workflow store service.
    #[must_use]
    pub fn new(store: Arc<dyn WorkflowStore>) -> Self {
        Self { svc: store }
    }

    /// Save a workflow definition.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowStoreError`] if the write fails.
    pub async fn save(
        &self,
        name: &str,
        def: &WorkflowDef,
    ) -> Result<(), Report<WorkflowStoreError>> {
        self.svc.save(name, def).await
    }

    /// Load a workflow definition by name.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowStoreError`] if the read or parse fails.
    pub async fn load(
        &self,
        name: &str,
    ) -> Result<Option<WorkflowDef>, Report<WorkflowStoreError>> {
        self.svc.load(name).await
    }

    /// List all saved workflow definitions.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowStoreError`] if the directory cannot be read.
    pub async fn list(&self) -> Result<Vec<(String, String)>, Report<WorkflowStoreError>> {
        self.svc.list().await
    }

    /// Delete a workflow definition by name.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowStoreError`] if the delete fails.
    pub async fn delete(&self, name: &str) -> Result<(), Report<WorkflowStoreError>> {
        self.svc.delete(name).await
    }
}

impl std::fmt::Debug for dyn WorkflowStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowStore")
            .field("name", &self.name())
            .finish()
    }
}

/// File-backed workflow definition store.
///
/// Reads and writes individual JSON files in `~/.config/nullslop/workflows/`.
/// Each file is named `<workflow-name>.json` and contains a single
/// [`WorkflowDef`] serialized as JSON.
pub struct FileWorkflowStore {
    /// Base directory containing workflow JSON files.
    dir: PathBuf,
}

impl Default for FileWorkflowStore {
    fn default() -> Self {
        Self::new()
    }
}

impl FileWorkflowStore {
    /// Creates a store at the platform config directory.
    ///
    /// Uses `dirs::config_dir()` → `nullslop/workflows/` on Linux.
    /// Does not create the directory until the first save.
    ///
    /// # Panics
    ///
    /// Panics if the platform config directory cannot be determined.
    #[expect(
        clippy::expect_used,
        reason = "platform config dir is always available on supported targets"
    )]
    #[must_use]
    pub fn new() -> Self {
        let dir = dirs::config_dir()
            .expect("platform config directory should be available")
            .join(DIR_NAME)
            .join(WORKFLOWS_DIR);
        Self { dir }
    }

    /// Creates a store at an explicit directory (for testing).
    #[must_use]
    pub fn new_in(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Returns the file path for a given workflow name.
    fn file_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.json"))
    }

    /// Ensures the directory exists, creating it if needed.
    async fn ensure_dir(&self) -> Result<(), Report<WorkflowStoreError>> {
        if !self.dir.exists() {
            tokio::fs::create_dir_all(&self.dir)
                .await
                .change_context(WorkflowStoreError)
                .attach("failed to create workflows directory")?;
        }
        Ok(())
    }
}

#[async_trait]
impl WorkflowStore for FileWorkflowStore {
    fn name(&self) -> &'static str {
        "file"
    }

    async fn save(&self, name: &str, def: &WorkflowDef) -> Result<(), Report<WorkflowStoreError>> {
        self.ensure_dir().await?;

        let path = self.file_path(name);
        let content = serde_json::to_string_pretty(def)
            .change_context(WorkflowStoreError)
            .attach("failed to serialize workflow definition")?;

        // Write to a temp file in the same directory, then rename (atomic).
        let tmp_path = path.with_extension("json.tmp");
        tokio::fs::write(&tmp_path, &content)
            .await
            .change_context(WorkflowStoreError)
            .attach("failed to write workflow file")?;

        tokio::fs::rename(&tmp_path, &path)
            .await
            .change_context(WorkflowStoreError)
            .attach("failed to rename workflow file")?;

        Ok(())
    }

    async fn load(&self, name: &str) -> Result<Option<WorkflowDef>, Report<WorkflowStoreError>> {
        let path = self.file_path(name);
        if !path.exists() {
            return Ok(None);
        }

        let content = tokio::fs::read_to_string(&path)
            .await
            .change_context(WorkflowStoreError)
            .attach("failed to read workflow file")?;

        let def: WorkflowDef = serde_json::from_str(&content)
            .change_context(WorkflowStoreError)
            .attach("failed to parse workflow definition")?;

        Ok(Some(def))
    }

    async fn list(&self) -> Result<Vec<(String, String)>, Report<WorkflowStoreError>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&self.dir)
            .await
            .change_context(WorkflowStoreError)
            .attach("failed to read workflows directory")?;

        while let Some(entry) = read_dir
            .next_entry()
            .await
            .change_context(WorkflowStoreError)
            .attach("failed to read directory entry")?
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }

            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_owned();

            if name.is_empty() {
                continue;
            }

            // Read and parse to extract the description.
            let content = match tokio::fs::read_to_string(&path).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        err = %e,
                        "skipping unreadable workflow file"
                    );
                    continue;
                }
            };

            match serde_json::from_str::<WorkflowDef>(&content) {
                Ok(def) => entries.push((name, def.description)),
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        err = %e,
                        "skipping unparseable workflow file"
                    );
                }
            }
        }

        // Sort by name for stable ordering.
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(entries)
    }

    async fn delete(&self, name: &str) -> Result<(), Report<WorkflowStoreError>> {
        let path = self.file_path(name);
        if path.exists() {
            tokio::fs::remove_file(&path)
                .await
                .change_context(WorkflowStoreError)
                .attach("failed to delete workflow file")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use nullslop_workflow::definition::{ModelHint, StepDef};
    use tempfile::TempDir;

    use super::*;

    /// Creates a minimal `WorkflowDef` for testing.
    fn make_def(name: &str, description: &str, step_count: usize) -> WorkflowDef {
        let steps: Vec<StepDef> = (0..step_count)
            .map(|i| StepDef {
                id: format!("step-{i}"),
                title: format!("Step {i}"),
                instructions: format!("Do thing {i}"),
                model_hint: ModelHint::Small,
                checkpoint: false,
                requires_user_input: false,
                tools: vec![],
                guards: nullslop_workflow::guard::GuardExpr::None,
                outputs: vec![],
                depends_on: vec![],
            })
            .collect();

        WorkflowDef {
            version: 1,
            name: name.to_owned(),
            description: description.to_owned(),
            model_overrides: HashMap::new(),
            globals: HashMap::new(),
            steps,
        }
    }

    // --- Test 1: Save + load round-trip ---

    #[tokio::test]
    async fn save_then_load_round_trips_workflow_definition() {
        // Given a FileWorkflowStore in a temp directory.
        let dir = TempDir::new().expect("temp dir");
        let store = FileWorkflowStore::new_in(dir.path().to_path_buf());
        let def = make_def("test-wf", "A test workflow", 2);

        // When saving and loading.
        store.save("test-wf", &def).await.expect("save");
        let loaded = store.load("test-wf").await.expect("load");

        // Then the loaded definition matches the saved one.
        let loaded = loaded.expect("should have a workflow");
        assert_eq!(loaded.name, "test-wf");
        assert_eq!(loaded.description, "A test workflow");
        assert_eq!(loaded.steps.len(), 2);
    }

    // --- Test 2: Load returns None for missing workflow ---

    #[tokio::test]
    async fn load_returns_none_for_missing_workflow() {
        // Given a FileWorkflowStore in a temp directory (empty).
        let dir = TempDir::new().expect("temp dir");
        let store = FileWorkflowStore::new_in(dir.path().to_path_buf());

        // When loading a nonexistent workflow.
        let result = store.load("nonexistent").await.expect("load");

        // Then None is returned.
        assert!(result.is_none());
    }

    // --- Test 3: List returns all saved workflows sorted by name ---

    #[tokio::test]
    async fn list_returns_correct_count() {
        // Given a FileWorkflowStore with 3 workflows saved.
        let dir = TempDir::new().expect("temp dir");
        let store = FileWorkflowStore::new_in(dir.path().to_path_buf());

        store
            .save("charlie", &make_def("charlie", "Third workflow", 1))
            .await
            .expect("save charlie");
        store
            .save("alpha", &make_def("alpha", "First workflow", 1))
            .await
            .expect("save alpha");
        store
            .save("bravo", &make_def("bravo", "Second workflow", 1))
            .await
            .expect("save bravo");

        // When calling list.
        let entries = store.list().await.expect("list");

        // Then 3 entries are returned.
        assert_eq!(entries.len(), 3);
    }

    #[tokio::test]
    async fn list_is_sorted_alphabetically() {
        // Given a FileWorkflowStore with 3 workflows saved.
        let dir = TempDir::new().expect("temp dir");
        let store = FileWorkflowStore::new_in(dir.path().to_path_buf());

        store
            .save("charlie", &make_def("charlie", "Third workflow", 1))
            .await
            .expect("save charlie");
        store
            .save("alpha", &make_def("alpha", "First workflow", 1))
            .await
            .expect("save alpha");
        store
            .save("bravo", &make_def("bravo", "Second workflow", 1))
            .await
            .expect("save bravo");

        // When calling list.
        let entries = store.list().await.expect("list");

        // Then entries are sorted alphabetically.
        assert_eq!(
            entries[0],
            ("alpha".to_owned(), "First workflow".to_owned())
        );
        assert_eq!(
            entries[1],
            ("bravo".to_owned(), "Second workflow".to_owned())
        );
        assert_eq!(
            entries[2],
            ("charlie".to_owned(), "Third workflow".to_owned())
        );
    }

    // --- Test 4: Delete removes workflow file ---

    #[tokio::test]
    async fn delete_returns_none_on_load() {
        // Given a FileWorkflowStore with a saved workflow.
        let dir = TempDir::new().expect("temp dir");
        let store = FileWorkflowStore::new_in(dir.path().to_path_buf());
        store
            .save("test-wf", &make_def("test-wf", "A test", 1))
            .await
            .expect("save");

        // When deleting the workflow.
        store.delete("test-wf").await.expect("delete");

        // Then load returns None.
        let result = store.load("test-wf").await.expect("load");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn delete_removes_file_from_disk() {
        // Given a FileWorkflowStore with a saved workflow.
        let dir = TempDir::new().expect("temp dir");
        let store = FileWorkflowStore::new_in(dir.path().to_path_buf());
        store
            .save("test-wf", &make_def("test-wf", "A test", 1))
            .await
            .expect("save");

        // When deleting the workflow.
        store.delete("test-wf").await.expect("delete");

        // Then the .json file no longer exists on disk.
        assert!(!dir.path().join("test-wf.json").exists());
    }

    // --- Test 5: Delete is idempotent for missing workflow ---

    #[tokio::test]
    async fn delete_succeeds_for_missing_workflow() {
        // Given a FileWorkflowStore in a temp directory (empty).
        let dir = TempDir::new().expect("temp dir");
        let store = FileWorkflowStore::new_in(dir.path().to_path_buf());

        // When deleting a nonexistent workflow.
        let result = store.delete("nonexistent").await;

        // Then Ok(()) is returned (no error).
        assert!(result.is_ok());
    }

    // --- Test 6: Save overwrites existing workflow ---

    #[tokio::test]
    async fn save_overwrites_existing_workflow() {
        // Given a FileWorkflowStore with a saved workflow (1 step).
        let dir = TempDir::new().expect("temp dir");
        let store = FileWorkflowStore::new_in(dir.path().to_path_buf());
        store
            .save("test-wf", &make_def("test-wf", "Original", 1))
            .await
            .expect("save v1");

        // When saving the same name with 3 steps.
        store
            .save("test-wf", &make_def("test-wf", "Updated", 3))
            .await
            .expect("save v2");

        // Then load returns the updated definition.
        let loaded = store.load("test-wf").await.expect("load").expect("some");
        assert_eq!(loaded.description, "Updated");
        assert_eq!(loaded.steps.len(), 3);
    }

    // --- Test 7: Save creates directory if missing ---

    #[tokio::test]
    async fn save_creates_directory_if_missing() {
        // Given a FileWorkflowStore pointed at a non-existent directory.
        let dir = TempDir::new().expect("temp dir");
        let nested = dir.path().join("does").join("not").join("exist");
        let store = FileWorkflowStore::new_in(nested.clone());
        let def = make_def("test-wf", "Mkdir test", 1);

        // When saving a workflow.
        store.save("test-wf", &def).await.expect("save");

        // Then the directory and file are created.
        assert!(nested.exists());
        assert!(nested.join("test-wf.json").exists());
    }

    // --- Test 8: List skips corrupted files ---

    #[tokio::test]
    async fn list_skips_corrupted_files_gracefully() {
        // Given a FileWorkflowStore directory with one valid and one corrupted file.
        let dir = TempDir::new().expect("temp dir");
        let store = FileWorkflowStore::new_in(dir.path().to_path_buf());

        // Save a valid workflow.
        store
            .save("valid-wf", &make_def("valid-wf", "A valid workflow", 1))
            .await
            .expect("save valid");

        // Write a corrupted file directly.
        tokio::fs::write(dir.path().join("corrupted.json"), "this is not json")
            .await
            .expect("write corrupted");

        // When calling list.
        let entries = store.list().await.expect("list");

        // Then only the valid workflow is returned.
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "valid-wf");
        assert_eq!(entries[0].1, "A valid workflow");
    }

    // --- Test 9: Service wrapper delegates to underlying store ---

    #[tokio::test]
    async fn service_wrapper_delegates_to_underlying_store() {
        // Given a WorkflowStoreService wrapping a FileWorkflowStore.
        let dir = TempDir::new().expect("temp dir");
        let store = FileWorkflowStore::new_in(dir.path().to_path_buf());
        let service = WorkflowStoreService::new(Arc::new(store));

        // When saving and loading via the service.
        let def = make_def("svc-test", "Service test", 1);
        service.save("svc-test", &def).await.expect("save");
        let loaded = service.load("svc-test").await.expect("load");

        // Then the data round-trips correctly through the service.
        let loaded = loaded.expect("should have a workflow");
        assert_eq!(loaded.name, "svc-test");
    }
}
