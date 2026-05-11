use super::*;

fn make_step(id: &str) -> StepDef {
    StepDef {
        id: id.to_owned(),
        title: format!("Step {id}"),
        instructions: format!("Instructions for {id}"),
        model_hint: ModelHint::Small,
        checkpoint: false,
        requires_user_input: false,
        tools: vec![],
        guards: GuardExpr::None,
        outputs: vec![],
        depends_on: vec![],
    }
}

#[rstest::rstest]
fn create_with_valid_name_succeeds() {
    // Given a new builder.
    let mut builder = WorkflowBuilder::new();

    // When creating with valid name and description.
    let result = builder.create("my-workflow".to_owned(), "A test".to_owned());

    // Then it succeeds.
    assert!(result.is_ok());
}

#[rstest::rstest]
fn create_with_empty_name_fails() {
    let mut builder = WorkflowBuilder::new();
    let result = builder.create(String::new(), "desc".to_owned());
    assert!(result.is_err());
}

#[rstest::rstest]
fn add_step_with_unique_id_succeeds() {
    let mut builder = WorkflowBuilder::new();
    builder
        .create("test".to_owned(), "desc".to_owned())
        .unwrap();

    let result = builder.add_step(make_step("step-1"));
    assert!(result.is_ok());
}

#[rstest::rstest]
fn add_step_with_duplicate_id_fails() {
    let mut builder = WorkflowBuilder::new();
    builder
        .create("test".to_owned(), "desc".to_owned())
        .unwrap();
    builder.add_step(make_step("step-1")).unwrap();

    let result = builder.add_step(make_step("step-1"));
    assert!(result.is_err());
}

#[rstest::rstest]
fn add_guard_for_existing_step_succeeds() {
    let mut builder = WorkflowBuilder::new();
    builder
        .create("test".to_owned(), "desc".to_owned())
        .unwrap();
    builder.add_step(make_step("step-1")).unwrap();

    let result = builder.add_guard(
        "step-1",
        GuardPredicate::FileExists {
            path: "/tmp/test".to_owned(),
        },
    );
    assert!(result.is_ok());
}

#[rstest::rstest]
fn add_guard_for_unknown_step_fails() {
    let mut builder = WorkflowBuilder::new();
    builder
        .create("test".to_owned(), "desc".to_owned())
        .unwrap();

    let result = builder.add_guard(
        "nope",
        GuardPredicate::FileExists {
            path: "/tmp/test".to_owned(),
        },
    );
    assert!(result.is_err());
}

#[rstest::rstest]
fn add_output_for_existing_step_succeeds() {
    let mut builder = WorkflowBuilder::new();
    builder
        .create("test".to_owned(), "desc".to_owned())
        .unwrap();
    builder.add_step(make_step("step-1")).unwrap();

    let result = builder.add_output(
        "step-1",
        StepOutputDef::File {
            label: "Output".to_owned(),
            path: "/tmp/out".to_owned(),
        },
    );
    assert!(result.is_ok());
}

#[rstest::rstest]
fn add_output_for_unknown_step_fails() {
    let mut builder = WorkflowBuilder::new();
    builder
        .create("test".to_owned(), "desc".to_owned())
        .unwrap();

    let result = builder.add_output(
        "nope",
        StepOutputDef::File {
            label: "Output".to_owned(),
            path: "/tmp/out".to_owned(),
        },
    );
    assert!(result.is_err());
}

#[rstest::rstest]
fn build_with_empty_steps_fails() {
    let mut builder = WorkflowBuilder::new();
    builder
        .create("test".to_owned(), "desc".to_owned())
        .unwrap();

    let result = builder.build();
    assert!(result.is_err());
}

/// Builds a valid workflow def for field checks.
fn build_valid_workflow_def() -> WorkflowDef {
    let mut builder = WorkflowBuilder::new();
    builder
        .create("my-workflow".to_owned(), "A test workflow".to_owned())
        .unwrap();
    builder.add_global("base_dir".to_owned(), "/tmp".to_owned());
    builder.add_step(make_step("step-1")).unwrap();
    builder.build().unwrap()
}

#[rstest::rstest]
#[case::name("name", "my-workflow")]
#[case::description("description", "A test workflow")]
fn build_with_valid_data_string_field_matches(#[case] field: &str, #[case] expected: &str) {
    // Given a valid workflow definition.
    let def = build_valid_workflow_def();

    // Then the field matches the expected value.
    let actual = match field {
        "name" => &def.name,
        "description" => &def.description,
        _ => panic!("unknown field: {field}"),
    };
    assert_eq!(actual, expected);
}

#[rstest::rstest]
fn build_with_valid_data_has_correct_globals_and_steps() {
    // Given a valid workflow definition.
    let def = build_valid_workflow_def();

    // Then globals and steps are correct.
    assert_eq!(def.globals.get("base_dir"), Some(&"/tmp".to_owned()));
    assert_eq!(def.steps.len(), 1);
    assert_eq!(def.steps.first().unwrap().id, "step-1");
}

#[rstest::rstest]
fn build_without_name_fails() {
    let builder = WorkflowBuilder::new();
    let result = builder.build();
    assert!(result.is_err());
}

/// Builds a workflow preview with checkpoint, user-input, guard, output, globals, and model overrides.
fn build_preview_workflow() -> String {
    let mut builder = WorkflowBuilder::new();
    builder
        .create("test-workflow".to_owned(), "A test".to_owned())
        .unwrap();
    builder.add_global("dir".to_owned(), "/tmp".to_owned());
    builder.set_model_override("small".to_owned(), "ollama/phi3".to_owned());

    let mut step = make_step("create-dir");
    step.checkpoint = true;
    step.requires_user_input = true;
    builder.add_step(step).unwrap();

    builder
        .add_guard(
            "create-dir",
            GuardPredicate::FileExists {
                path: "{{dir}}/notes.md".to_owned(),
            },
        )
        .unwrap();

    builder
        .add_output(
            "create-dir",
            StepOutputDef::File {
                label: "Notes".to_owned(),
                path: "{{dir}}/notes.md".to_owned(),
            },
        )
        .unwrap();

    builder.preview()
}

#[rstest::rstest]
#[case::workflow_name("Workflow: test-workflow")]
#[case::step_id("create-dir")]
#[case::checkpoint("checkpoint")]
#[case::user_input("user-input")]
#[case::guard("file_exists({{dir}}/notes.md)")]
#[case::output("Notes (file)")]
#[case::globals("Globals: dir")]
#[case::model_overrides("Model overrides: small → ollama/phi3")]
fn preview_produces_expected_format(#[case] expected: &str) {
    // Given a workflow builder with a checkpoint step, guard, output, globals, and model overrides.
    let preview = build_preview_workflow();

    // Then the preview contains the expected text.
    assert!(
        preview.contains(expected),
        "preview should contain {expected:?}, got: {preview}"
    );
}

#[rstest::rstest]
fn preview_shows_all_pieces() {
    // Given a builder.
    let mut builder = WorkflowBuilder::new();

    // When building a complete workflow.
    builder
        .create(
            "video-workflow".to_owned(),
            "Music video workflow".to_owned(),
        )
        .unwrap();

    builder.add_global("video_dir".to_owned(), "/tmp/video".to_owned());
    builder.set_model_override("small".to_owned(), "ollama/phi3".to_owned());

    builder.add_step(make_step("setup")).unwrap();
    builder.add_step(make_step("render")).unwrap();

    builder
        .add_guard(
            "setup",
            GuardPredicate::FileExists {
                path: "{{video_dir}}/config.json".to_owned(),
            },
        )
        .unwrap();

    builder
        .add_output(
            "setup",
            StepOutputDef::Summary {
                label: "Config".to_owned(),
                value: "done".to_owned(),
            },
        )
        .unwrap();

    // Then preview shows all the pieces.
    let preview = builder.preview();
    assert!(preview.contains("video-workflow"));
    assert!(preview.contains("setup"));
    assert!(preview.contains("render"));
    assert!(preview.contains("file_exists({{video_dir}}/config.json)"));
    assert!(preview.contains("Config (summary)"));
}

#[rstest::rstest]
fn build_produces_valid_def() {
    // Given a builder.
    let mut builder = WorkflowBuilder::new();

    // When building a complete workflow.
    builder
        .create(
            "video-workflow".to_owned(),
            "Music video workflow".to_owned(),
        )
        .unwrap();

    builder.add_global("video_dir".to_owned(), "/tmp/video".to_owned());
    builder.set_model_override("small".to_owned(), "ollama/phi3".to_owned());

    builder.add_step(make_step("setup")).unwrap();
    builder.add_step(make_step("render")).unwrap();

    builder
        .add_guard(
            "setup",
            GuardPredicate::FileExists {
                path: "{{video_dir}}/config.json".to_owned(),
            },
        )
        .unwrap();

    builder
        .add_output(
            "setup",
            StepOutputDef::Summary {
                label: "Config".to_owned(),
                value: "done".to_owned(),
            },
        )
        .unwrap();

    // Then build produces a valid WorkflowDef.
    let def = builder.build().unwrap();
    assert_eq!(def.name, "video-workflow");
    assert_eq!(def.steps.len(), 2);
    assert_eq!(
        def.steps.first().unwrap().guards,
        GuardExpr::Predicate(GuardPredicate::FileExists {
            path: "{{video_dir}}/config.json".to_owned(),
        })
    );
}

#[rstest::rstest]
fn adding_multiple_guards_combines_with_all() {
    let mut builder = WorkflowBuilder::new();
    builder
        .create("test".to_owned(), "desc".to_owned())
        .unwrap();
    builder.add_step(make_step("step-1")).unwrap();

    builder
        .add_guard(
            "step-1",
            GuardPredicate::FileExists {
                path: "/a".to_owned(),
            },
        )
        .unwrap();

    builder
        .add_guard(
            "step-1",
            GuardPredicate::FileExists {
                path: "/b".to_owned(),
            },
        )
        .unwrap();

    let def = builder.build().unwrap();
    let step = def.steps.first().unwrap();

    // Should be wrapped in All.
    match &step.guards {
        GuardExpr::All { all } => {
            assert_eq!(all.len(), 2);
        }
        other => panic!("expected All, got {other:?}"),
    }
}

// --- validate() tests ---

#[rstest::rstest]
fn validate_succeeds_for_valid_draft() {
    // Given a builder with name, description, and a step.
    let mut builder = WorkflowBuilder::new();
    builder
        .create("my-workflow".to_owned(), "A test".to_owned())
        .unwrap();
    builder.add_step(make_step("step-1")).unwrap();

    // When validating.
    let result = builder.validate();

    // Then it succeeds.
    assert!(result.is_ok());
}

#[rstest::rstest]
fn validate_fails_without_name() {
    // Given a builder with no name.
    let builder = WorkflowBuilder::new();

    // When validating.
    let result = builder.validate();

    // Then it fails with MissingField("name").
    let err = result.expect_err("should fail");
    let kind = err.current_context().kind();
    assert_eq!(*kind, WorkflowErrorKind::MissingField("name".to_owned()));
}

#[rstest::rstest]
fn validate_fails_with_no_steps() {
    // Given a builder with name and description but no steps.
    let mut builder = WorkflowBuilder::new();
    builder
        .create("my-workflow".to_owned(), "A test".to_owned())
        .unwrap();

    // When validating.
    let result = builder.validate();

    // Then it fails with EmptyWorkflow.
    let err = result.expect_err("should fail");
    assert_eq!(
        *err.current_context().kind(),
        WorkflowErrorKind::EmptyWorkflow
    );
}

#[rstest::rstest]
fn validate_does_not_consume_builder() {
    // Given a builder with name, description, and a step.
    let mut builder = WorkflowBuilder::new();
    builder
        .create("test".to_owned(), "desc".to_owned())
        .unwrap();
    builder.add_step(make_step("s1")).unwrap();

    // When validating.
    builder.validate().unwrap();

    // Then the builder is still usable.
    builder.add_step(make_step("s2")).unwrap();
    let def = builder.build().unwrap();
    assert_eq!(def.steps.len(), 2);
}
