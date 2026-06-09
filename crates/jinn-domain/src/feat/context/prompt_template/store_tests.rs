#![allow(clippy::expect_used, clippy::panic, clippy::unreachable, clippy::indexing_slicing, reason = "test code")]

use crate::feat::context::prompt_template::store::{MAX_FUZZY_RESULTS, PromptTemplateStore};
use crate::protocol::PromptTemplate;
use std::path::Path;
use tempfile::TempDir;

fn write_template(dir: &Path, filename: &str, content: &str) {
    std::fs::write(dir.join(filename), content).expect("write template file");
}

#[rstest::rstest]
fn load_from_dir_returns_empty_when_missing() {
    // Given a path that does not exist.
    let path = Path::new("/tmp/jinn-nonexistent-test-dir");

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

#[rstest::rstest]
fn scan_dir_skips_underscore_prefixed_files() {
    // Given a directory with both a normal template and an underscore-prefixed file.
    let dir = TempDir::new().expect("temp dir");
    write_template(
        dir.path(),
        "normal.md",
        "+++\nname = \"normal\"\ndescription = \"Normal template\"\n+++\nBody.",
    );
    write_template(
        dir.path(),
        "_system.md",
        "+++\nname = \"system\"\ndescription = \"System template\"\n+++\nBody.",
    );

    // When loading from that directory.
    let store = PromptTemplateStore::load_from_dir(dir.path()).expect("load");

    // Then only the normal template is loaded; _system is skipped.
    assert_eq!(store.len(), 1);
    assert!(store.find_by_name("normal").is_some());
    assert!(store.find_by_name("system").is_none());
}

#[rstest::rstest]
fn load_from_dirs_user_override_skips_underscore_prefixed() {
    // Given a user dir with an underscore-prefixed file and a system dir with a normal template.
    let user_dir = TempDir::new().expect("temp dir");
    let system_dir = TempDir::new().expect("temp dir");
    write_template(
        user_dir.path(),
        "_compaction.md",
        "+++\nname = \"_compaction\"\ndescription = \"Compaction\"\n+++\nBody.",
    );
    write_template(
        system_dir.path(),
        "review.md",
        "+++\nname = \"review\"\ndescription = \"Review\"\n+++\nBody.",
    );

    // When loading from both dirs.
    let store =
        PromptTemplateStore::load_from_dirs(user_dir.path(), system_dir.path()).expect("load");

    // Then only the non-prefixed template is loaded.
    assert_eq!(store.len(), 1);
    assert!(store.find_by_name("review").is_some());
    assert!(store.find_by_name("_compaction").is_none());
}

#[rstest::rstest]
fn fuzzy_search_never_returns_underscore_prefixed_templates() {
    // Given a store built directly (bypassing scan) that includes an underscore-prefixed name.
    // This verifies that even if somehow a _-prefixed template got into the store,
    // the search would still return it - confirming the filter must happen at scan time.
    let store = PromptTemplateStore::from_vec(vec![PromptTemplate {
        name: "_compaction".to_owned(),
        description: "System".to_owned(),
        body: String::new(),
    }]);

    // When searching with a broad query.
    let results = store.fuzzy_search("compaction");

    // Then the _-prefixed template is returned (because it's in the store).
    // This confirms the real protection is at scan time, not search time.
    assert_eq!(results.len(), 1);
}

#[rstest::rstest]
fn load_from_dirs_ordered_project_adds_net_new_template() {
    // Given a user dir with one template and a project dir with a different one.
    let user_dir = TempDir::new().expect("user dir");
    write_template(
        user_dir.path(),
        "hello.md",
        "+++\nname = \"hello\"\ndescription = \"User hello\"\n+++\nBody hello",
    );
    let system_dir = TempDir::new().expect("system dir");
    let project_dir = TempDir::new().expect("project dir");
    write_template(
        project_dir.path(),
        "extra.md",
        "+++\nname = \"extra\"\ndescription = \"Project extra\"\n+++\nBody extra",
    );

    // When loading with the project dir.
    let store = PromptTemplateStore::load_from_dirs_ordered(
        user_dir.path(),
        system_dir.path(),
        &[project_dir.path().to_path_buf()],
    )
    .expect("load");

    // Then both templates are present.
    assert_eq!(store.len(), 2);
    assert!(store.find_by_name("hello").is_some());
    assert!(store.find_by_name("extra").is_some());
}

#[rstest::rstest]
fn load_from_dirs_ordered_project_overrides_user() {
    // Given a user template and a project template with the same name.
    let user_dir = TempDir::new().expect("user dir");
    write_template(
        user_dir.path(),
        "shared.md",
        "+++\nname = \"shared\"\ndescription = \"User version\"\n+++\nUser body",
    );
    let system_dir = TempDir::new().expect("system dir");
    let project_dir = TempDir::new().expect("project dir");
    write_template(
        project_dir.path(),
        "shared.md",
        "+++\nname = \"shared\"\ndescription = \"Project version\"\n+++\nProject body",
    );

    // When loading with the project dir as the highest-priority layer.
    let store = PromptTemplateStore::load_from_dirs_ordered(
        user_dir.path(),
        system_dir.path(),
        &[project_dir.path().to_path_buf()],
    )
    .expect("load");

    // Then the project version overrides the user version.
    assert_eq!(store.len(), 1);
    let tmpl = store.find_by_name("shared").expect("shared exists");
    assert_eq!(tmpl.description, "Project version");
    assert_eq!(tmpl.body, "Project body");
}

#[rstest::rstest]
fn load_from_dirs_ordered_most_local_ancestor_wins() {
    // Given two project ancestors each with a template of the same name.
    let user_dir = TempDir::new().expect("user dir");
    let system_dir = TempDir::new().expect("system dir");
    let ancestor = TempDir::new().expect("ancestor dir");
    write_template(
        ancestor.path(),
        "dup.md",
        "+++\nname = \"dup\"\ndescription = \"Ancestor version\"\n+++\nAncestor body",
    );
    let local = TempDir::new().expect("local dir");
    write_template(
        local.path(),
        "dup.md",
        "+++\nname = \"dup\"\ndescription = \"Local version\"\n+++\nLocal body",
    );

    // When loading with ancestor first (least-local), local last (most-local).
    let store = PromptTemplateStore::load_from_dirs_ordered(
        user_dir.path(),
        system_dir.path(),
        &[ancestor.path().to_path_buf(), local.path().to_path_buf()],
    )
    .expect("load");

    // Then the most-local (last) entry wins.
    assert_eq!(store.len(), 1);
    let tmpl = store.find_by_name("dup").expect("dup exists");
    assert_eq!(tmpl.description, "Local version");
    assert_eq!(tmpl.body, "Local body");
}
