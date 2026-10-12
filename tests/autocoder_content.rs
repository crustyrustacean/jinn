//! Autocoder workflow content-loading checks.
//!
//! The workflow is data — a workflow directory plus attendant TOML files.
//! These tests load that content through the real production paths:
//! [`jinn_workflows::scan_workflows`] discovers the workflow (frontmatter
//! admission included), and the attendant files deserialize through the real
//! [`AttendantEntryConfig`] schema, exactly as `load_attendant` parses
//! them. A typo in shipped content fails here, not at the owner's first
//! run.

use std::path::PathBuf;
use toml::from_str as parse_toml;

/// The shipped autocoder workflow directory, under the repo's `res/` tree.
fn autocoder_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("res/workflows/autocoder")
}

#[rstest::rstest]
#[test]
fn autocoder_workflow_admits_through_real_discovery() {
    // Given the shipped autocoder workflow directory and its parent.
    let parent = autocoder_dir()
        .parent()
        .expect("autocoder has a workflows parent")
        .to_path_buf();

    // When scanning through the real discovery path.
    let found: Vec<jinn_workflows_msg::Workflow> = jinn_workflows::scan_workflows(&parent);

    // Then the autocoder admits: frontmatter present, description
    // non-blank, directory resolved.
    let autocoder = found
        .iter()
        .find(|w| w.dir == autocoder_dir())
        .expect("autocoder workflow must be discovered");
    assert_eq!(autocoder.name, "autocoder");
    assert!(
        autocoder.description.contains("staged attendant sessions"),
        "description carries the run's shape: {}",
        autocoder.description
    );
}

#[rstest::rstest]
#[test]
fn autocoder_steps_and_flow_lines_correspond() {
    // Given the shipped workflow.md and its steps directory.
    let dir = autocoder_dir();
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
fn attendant_files_deserialize_through_the_saved_entry_schema() {
    // Given the shipped attendant TOMLs.
    let attendants = std::fs::read_dir(autocoder_dir().join("attendants"))
        .expect("attendants directory")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
        .count();

    // When deserializing each through the production schema
    // `load_attendant` uses — the files carry the real document shape
    // (`[[attendant.entry]]`), so the wrapper mirrors that shape.
    #[derive(serde::Deserialize)]
    struct AttendantFile {
        attendant: AttendantSection,
    }
    #[derive(serde::Deserialize)]
    struct AttendantSection {
        entry: Vec<jinn_preferences_config::schemas::attendant::AttendantEntryConfig>,
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(autocoder_dir().join("attendants")).expect("attendants") {
        let entry = entry.expect("dir entry");
        if entry.path().extension().is_some_and(|x| x != "toml") {
            continue;
        }
        let text = std::fs::read_to_string(entry.path()).expect("attendant file");
        let file: AttendantFile = parse_toml(&text)
            .unwrap_or_else(|e| panic!("{} does not deserialize: {e}", entry.path().display()));
        assert_eq!(
            file.attendant.entry.len(),
            1,
            "{} declares exactly one attendant",
            entry.path().display()
        );
        let config = &file.attendant.entry[0];
        assert!(
            !config.name.is_empty(),
            "{} carries a name",
            entry.path().display()
        );
        names.push(config.name.clone());
    }

    // Then the shipped set is the thin-seed trio.
    assert_eq!(attendants, 3, "planner, implementer, reviewer");
    for expected in ["planner", "implementer", "reviewer"] {
        assert!(
            names.iter().any(|n| n == expected),
            "the shipped attendants must include {expected}, got {names:?}"
        );
    }
}

#[rstest::rstest]
#[test]
fn lifecycle_snippet_deserializes_through_the_lifecycle_schema() {
    // Given the shipped lifecycle snippet.
    let text =
        std::fs::read_to_string(autocoder_dir().join("lifecycle.toml")).expect("lifecycle.toml");

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

    // Then all the autocoder scripts ship, every command uses named
    // placeholders only, and no script writes trunk or deletes a
    // variable path outside the trash mechanism.
    let names: Vec<&str> = scripts.iter().map(|s| s.name.as_str()).collect();
    for expected in [
        "autocoder-fossil-root",
        "autocoder-fossil-stage",
        "autocoder-trash-stage",
        "autocoder-trash-purge",
        "autocoder-git-root",
        "autocoder-git-stage",
    ] {
        assert!(names.contains(&expected), "missing script {expected}");
    }
    let find = |name: &str| {
        scripts
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("missing script {name}"))
    };

    // The purge is un-aimable: no placeholders, and `rm` names exactly
    // one fixed path.
    let purge = find("autocoder-trash-purge");
    let jinn_preferences_config::schemas::session_lifecycle::LifecycleCommand::Shell(purge_setup) =
        purge.setup.as_ref().expect("purge setup exists")
    else {
        panic!("purge setup is a shell command");
    };
    assert!(
        !purge_setup.contains('<'),
        "the purge carries no placeholders, so the agent cannot aim it: {purge_setup}"
    );
    assert!(
        purge_setup.contains(".jinn-autocoder-trash"),
        "the purge deletes the fixed trash path: {purge_setup}"
    );

    // The guarded move is the only command that ever relocates a
    // variable path, and it is gated: checkout marker, repo file
    // match, and a worktrees/ prefix — with a refusal branch.
    let trash = find("autocoder-trash-stage");
    let jinn_preferences_config::schemas::session_lifecycle::LifecycleCommand::Shell(trash_setup) =
        trash.setup.as_ref().expect("trash setup exists")
    else {
        panic!("trash setup is a shell command");
    };
    for gate in [".fslckout", "grep -qF", "grep -q '^worktrees/'", "|| false"] {
        assert!(
            trash_setup.contains(gate),
            "the guarded move checks {gate}: {trash_setup}"
        );
    }
    assert!(
        trash_setup.contains("mv") && !trash_setup.contains("rm "),
        "the guarded move relocates, never deletes: {trash_setup}"
    );

    // No script anywhere merges to trunk or writes trunk; the only
    // `rm` in the whole file is the purge's fixed path.
    for script in &scripts {
        for command in [script.setup.as_ref(), script.teardown.as_ref()]
            .into_iter()
            .flatten()
        {
            let jinn_preferences_config::schemas::session_lifecycle::LifecycleCommand::Shell(text) =
                command
            else {
                continue;
            };
            assert!(
                !text.contains("fossil merge"),
                "{} must never merge — merging is a workflow step, not a script: {text}",
                script.name
            );
            if let Some(pos) = text.find("rm ") {
                assert!(
                    script.name == "autocoder-trash-purge"
                        && text[pos..].contains(".jinn-autocoder-trash"),
                    "only the purge deletes, and only the fixed trash path: {} -> {text}",
                    script.name
                );
            }
        }
        if script.name == "autocoder-fossil-stage" {
            let inert = matches!(
                script.teardown.as_ref(),
                Some(jinn_preferences_config::schemas::session_lifecycle::LifecycleCommand::Shell(
                    t
                )) if t == "true"
            );
            assert!(
                inert,
                "fossil stage teardown is inert (retirement is the orchestrator's guarded move)"
            );
        }
    }
}

#[rstest::rstest]
#[test]
fn stage_prompts_never_instruct_sibling_file_reads() {
    // Given every step file in the autocoder workflow.
    let dir = autocoder_dir().join("steps");

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

#[rstest::rstest]
#[test]
fn lifecycle_prose_loads_by_path_and_never_by_paste() {
    // Given every shipped autocoder prompt file that names the lifecycle
    // tool: workflow.md, the stage step, and the implementer attendant.
    let dir = autocoder_dir();
    let prose_files = [
        dir.join("workflow.md"),
        dir.join("steps/03-implement.md"),
        dir.join("attendants/implementer.toml"),
    ];

    // When reading them all.
    for path in &prose_files {
        let text = std::fs::read_to_string(path).expect("prose file");

        // Then the path form is present — the tool is called by its wire
        // name with the workflow's own lifecycle.toml as the source — and
        // no paste instruction or old tool name survives anywhere in the
        // workflow's content.
        assert!(
            text.contains("run_lifecycle"),
            "{} names the lifecycle tool by its wire name",
            path.display()
        );
        assert!(
            text.contains("lifecycle.toml"),
            "{} points at the workflow's lifecycle.toml",
            path.display()
        );
        assert!(
            !text.contains("run_lifecycle_script"),
            "{} must not carry the retired tool name",
            path.display()
        );
        assert!(
            !text.to_lowercase().contains("paste"),
            "{} must not instruct pasting scripts into jinn.toml",
            path.display()
        );
    }

    // And the lifecycle.toml header documents path loading, not pasting.
    let header = std::fs::read_to_string(dir.join("lifecycle.toml")).expect("lifecycle.toml");
    assert!(
        header.contains("loaded by path"),
        "the header documents path loading: {}",
        header.lines().take(10).collect::<Vec<_>>().join("\n")
    );
}
