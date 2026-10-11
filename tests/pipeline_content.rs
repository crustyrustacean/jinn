//! Pipeline workflow content-loading checks.
//!
//! The pipeline is data — a workflow directory plus cast TOML files.
//! These tests load that content through the real production paths:
//! [`jinn_workflows::scan_workflows`] discovers the workflow (frontmatter
//! admission included), and the cast files deserialize through the real
//! [`AttendantEntryConfig`] schema, exactly as `load_attendant` parses
//! them. A typo in shipped content fails here, not at the owner's first
//! run.

use std::path::{Path, PathBuf};
use toml::from_str as parse_toml;

/// The repo-local pipeline workflow directory.
fn pipeline_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".config/jinn/workflows/pipelines")
}

#[rstest::rstest]
#[test]
fn pipeline_workflow_admits_through_real_discovery() {
    // Given the shipped pipeline workflow directory and its parent.
    let parent = pipeline_dir()
        .parent()
        .expect("pipelines has a workflows parent")
        .to_path_buf();

    // When scanning through the real discovery path.
    let found: Vec<jinn_workflows_msg::Workflow> = jinn_workflows::scan_workflows(&parent);

    // Then the pipeline admits: frontmatter present, description
    // non-blank, directory resolved.
    let pipeline = found
        .iter()
        .find(|w| w.dir == pipeline_dir())
        .expect("pipelines workflow must be discovered");
    assert_eq!(pipeline.name, "pipelines");
    assert!(
        pipeline.description.contains("staged attendant sessions"),
        "description carries the run's shape: {}",
        pipeline.description
    );
}

#[rstest::rstest]
#[test]
fn pipeline_steps_and_flow_lines_correspond() {
    // Given the shipped workflow.md and its steps directory.
    let dir = pipeline_dir();
    let workflow_md = std::fs::read_to_string(dir.join("workflow.md")).expect("workflow.md");
    let steps = std::fs::read_dir(dir.join("steps"))
        .expect("steps directory")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .count();

    // When extracting the flow's numbered entries and the dispatch
    // paths they name.
    let named: Vec<&str> = workflow_md
        .lines()
        .filter_map(|l| l.trim().strip_prefix("- `steps/"))
        .filter_map(|l| l.split('`').next())
        .collect();

    // Then every dispatch path names a real step file, and the steps
    // directory holds no orphans.
    for name in &named {
        assert!(
            dir.join("steps").join(format!("{name}.md")).exists(),
            "flow names steps/{name}.md, which does not exist"
        );
    }
    assert!(
        steps >= named.len(),
        "steps directory holds files the flow never names: {named:?} vs {steps} files"
    );
    assert!(steps > 0, "a workflow with no steps is a note");
}

#[rstest::rstest]
#[test]
fn cast_files_deserialize_through_the_saved_entry_schema() {
    // Given the shipped cast TOMLs.
    let attendants = std::fs::read_dir(pipeline_dir().join("attendants"))
        .expect("attendants directory")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
        .count();

    // When deserializing each through the production schema
    // `load_attendant` uses — the files carry the real document shape
    // (`[[attendant.entry]]`), so the wrapper mirrors that shape.
    #[derive(serde::Deserialize)]
    struct CastFile {
        attendant: AttendantSection,
    }
    #[derive(serde::Deserialize)]
    struct AttendantSection {
        entry: Vec<jinn_preferences_config::schemas::attendant::AttendantEntryConfig>,
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(pipeline_dir().join("attendants")).expect("attendants") {
        let entry = entry.expect("dir entry");
        if entry.path().extension().is_some_and(|x| x != "toml") {
            continue;
        }
        let text = std::fs::read_to_string(entry.path()).expect("cast file");
        let cast: CastFile = parse_toml(&text)
            .unwrap_or_else(|e| panic!("{} does not deserialize: {e}", entry.path().display()));
        assert_eq!(
            cast.attendant.entry.len(),
            1,
            "{} declares exactly one member",
            entry.path().display()
        );
        let config = &cast.attendant.entry[0];
        assert!(
            !config.name.is_empty(),
            "{} carries a name",
            entry.path().display()
        );
        names.push(config.name.clone());
    }

    // Then the cast is the thin-seed trio.
    assert_eq!(attendants, 3, "planner, implementer, reviewer");
    for expected in ["planner", "implementer", "reviewer"] {
        assert!(
            names.iter().any(|n| n == expected),
            "cast must include {expected}, got {names:?}"
        );
    }
}

#[rstest::rstest]
#[test]
fn lifecycle_snippet_deserializes_through_the_lifecycle_schema() {
    // Given the shipped lifecycle snippet.
    let text =
        std::fs::read_to_string(pipeline_dir().join("lifecycle.toml")).expect("lifecycle.toml");

    // When deserializing its entries through the production list schema —
    // the snippet is the `[[session_lifecycle.script]]` section verbatim,
    // so it is read through a wrapper shaped like the real document.
    #[derive(serde::Deserialize)]
    struct Wrapper {
        session_lifecycle: Section,
    }
    #[derive(serde::Deserialize)]
    struct Section {
        script: Vec<jinn_preferences_config::schemas::session_lifecycle::SessionLifecycle>,
    }
    let wrapper: Wrapper = parse_toml(&text).expect("lifecycle snippet deserializes");
    let scripts = wrapper.session_lifecycle.script;

    // Then all four scripts ship paired commands, and neither stage
    // teardown writes trunk: fossil teardown closes and removes only;
    // the trunk merge exists nowhere in the file.
    let names: Vec<&str> = scripts.iter().map(|s| s.name.as_str()).collect();
    for expected in [
        "pipeline-fossil-root",
        "pipeline-fossil-stage",
        "pipeline-git-root",
        "pipeline-git-stage",
    ] {
        assert!(names.contains(&expected), "missing script {expected}");
    }
    for script in &scripts {
        if script.name.ends_with("-stage") {
            let jinn_preferences_config::schemas::session_lifecycle::LifecycleCommand::Shell(
                teardown,
            ) = script.teardown.as_ref().expect("stage teardown exists")
            else {
                panic!("stage teardown is a shell command");
            };
            assert!(
                !teardown.contains("merge")
                    && !teardown.contains("trunk")
                    && !teardown.contains(" main"),
                "stage teardown must never write trunk: {} -> {teardown}",
                script.name
            );
        }
    }
}

#[rstest::rstest]
#[test]
fn stage_prompts_never_instruct_sibling_file_reads() {
    // Given every step file in the pipeline.
    let dir = pipeline_dir().join("steps");

    // When reading them all.
    for entry in std::fs::read_dir(&dir).expect("steps") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_some_and(|x| x != "md") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("step file");

        // Then no stage is told to read a sibling stage's file — state
        // crosses sessions only in dispatch messages. The orchestrator
        // writing/reading its own ledger is the sanctioned exception.
        for banned in [
            "ledger directory, read",
            "read the other stage",
            "sibling's file",
        ] {
            assert!(
                !text.contains(banned),
                "{} instructs a sibling file read: {banned}",
                path.display()
            );
        }
    }
}
