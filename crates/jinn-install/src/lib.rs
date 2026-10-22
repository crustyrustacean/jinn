//! Default resource installation — seeds themes, personas, prompts, and
//! skills into the user's config and agent directories.
//!
//! Every resource under `res/` is embedded at compile time (see
//! [`bundled`]), so the binary is self-contained.
//!
//! [`install_defaults_to`] (`jinn install`) is a pure seeder. Payload files
//! follow skip/`--force` rules; `jinn.toml` and the stream-rules file are
//! written **exactly once**, only when they do not exist. An existing file
//! is never read or modified — even with `--force` — so user edits survive
//! and a malformed file never fails the install.

mod bundled;

use error_stack::{Report, ResultExt};
use jinn_preferences_config::StreamRulesSeedOutcome;
use marked_path::{Absolute, MarkedPath, MarkedPathAccess, MarkedPathBuf};
use wherror::Error;

use bundled::{Bundled, bundled_catalogue};

/// Relative destinations for the five resource kinds.
///
/// Each field is a root directory: themes/personas/prompts/workflows live
/// under the config dir (`~/.config/jinn`), skills live under the agent dir
/// (`~/.agents/skills`). Passed by value into [`install_defaults_to`] so
/// tests can point at temp dirs.
#[derive(Debug, Clone)]
pub struct Destinations {
    themes: MarkedPathBuf<Absolute>,
    personas: MarkedPathBuf<Absolute>,
    prompts: MarkedPathBuf<Absolute>,
    workflows: MarkedPathBuf<Absolute>,
    skills: MarkedPathBuf<Absolute>,
}

impl Destinations {
    /// Creates a destination set from the five root directories.
    #[must_use]
    pub fn new(
        themes: MarkedPath<Absolute>,
        personas: MarkedPath<Absolute>,
        prompts: MarkedPath<Absolute>,
        workflows: MarkedPath<Absolute>,
        skills: MarkedPath<Absolute>,
    ) -> Self {
        Self {
            themes: themes.to_owned(),
            personas: personas.to_owned(),
            prompts: prompts.to_owned(),
            workflows: workflows.to_owned(),
            skills: skills.to_owned(),
        }
    }

    /// The root directory themes install into.
    pub(crate) fn themes(&self) -> MarkedPath<'_, Absolute> {
        self.themes.as_marked_path()
    }

    /// The root directory personas install into.
    pub(crate) fn personas(&self) -> MarkedPath<'_, Absolute> {
        self.personas.as_marked_path()
    }

    /// The root directory prompts install into.
    pub(crate) fn prompts(&self) -> MarkedPath<'_, Absolute> {
        self.prompts.as_marked_path()
    }

    /// The root directory workflows install into — one subdirectory per
    /// workflow, the shape `scan_workflows` discovers.
    pub(crate) fn workflows(&self) -> MarkedPath<'_, Absolute> {
        self.workflows.as_marked_path()
    }

    /// The root directory skills install into.
    pub(crate) fn skills(&self) -> MarkedPath<'_, Absolute> {
        self.skills.as_marked_path()
    }
}

/// Outcome of installing one resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallOutcome {
    /// Resource was written to a previously-missing path.
    Created(MarkedPathBuf<Absolute>),
    /// Resource already existed and was left untouched.
    Skipped(MarkedPathBuf<Absolute>),
    /// Resource already existed and was overwritten in place.
    Overwritten(MarkedPathBuf<Absolute>),
}

impl InstallOutcome {
    /// The full destination path this outcome refers to.
    #[must_use]
    pub fn path(&self) -> &MarkedPathBuf<Absolute> {
        match self {
            InstallOutcome::Created(p)
            | InstallOutcome::Skipped(p)
            | InstallOutcome::Overwritten(p) => p,
        }
    }
}

/// Error returned by [`install_defaults_to`].
#[derive(Debug, Error)]
#[error(debug)]
pub struct InstallError;

/// The result of a full default install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport {
    /// Per-resource outcomes in catalogue order (deterministic).
    pub outcomes: Vec<InstallOutcome>,
    /// Outcome for the user preferences file itself.
    pub jinn_toml: JinnTomlOutcome,
    /// Outcome for the shipped stream-rules file, written once next to
    /// `jinn.toml`.
    pub stream_rules: StreamRulesSeedOutcome,
}

/// Outcome for `jinn.toml` during a default install.
///
/// `jinn install` writes the file exactly once — only when it does not
/// exist. An existing file is never read or modified, even under `--force`:
/// the user's `enabled`, `config`, and hand-edited grants always win.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JinnTomlOutcome {
    /// The file did not exist and was created this run (the comment-rich
    /// default template).
    Created(MarkedPathBuf<Absolute>),
    /// The file already existed and was never read or modified this run.
    Untouched(MarkedPathBuf<Absolute>),
}

/// Installs every bundled default resource into the given destinations.
///
/// Per resource:
/// - If the destination already exists and `overwrite` is false → [`InstallOutcome::Skipped`].
/// - If the destination already exists and `overwrite` is true → the file is replaced, yielding [`InstallOutcome::Overwritten`].
/// - Otherwise → parents are created via `create_dir_all` and the file is
///   written, yielding [`InstallOutcome::Created`].
///
/// Parents are created unconditionally before each write so a missing config
/// tree never surfaces as a "directory does not exist" write error.
///
/// **`jinn.toml` is written exactly once — only when it does not exist.**
/// If the file already exists it is never read or modified — even with
/// `overwrite` — so a malformed `jinn.toml` never fails the install; the
/// caller surfaces this via [`JinnTomlOutcome::Untouched`].
///
/// Outcomes are returned in catalogue order (deterministic), alongside the
/// `jinn.toml` outcome.
///
/// # Errors
///
/// Returns [`Report<InstallError>`] if directory creation or file writing
/// fails, or if preferences cannot be created on the fresh-create path.
///
/// # Panics
///
/// Panics if the preferences path has no parent, which cannot happen for an
/// absolute path: an absolute path always names a parent directory (the
/// filesystem root is its own).
#[expect(
    clippy::expect_used,
    reason = "parent of an absolute path is absolute by construction"
)]
pub fn install_defaults_to(
    destinations: &Destinations,
    overwrite: bool,
    prefs_path: &MarkedPathBuf<Absolute>,
) -> Result<InstallReport, Report<InstallError>> {
    let prefs_existed = prefs_path.exists();

    let catalogue = bundled_catalogue()?;

    let mut outcomes = Vec::with_capacity(catalogue.len());
    for resource in &catalogue {
        outcomes.push(install_resource(resource, destinations, overwrite)?);
    }

    let jinn_toml = if prefs_existed {
        JinnTomlOutcome::Untouched(prefs_path.to_owned())
    } else {
        // Fresh-create path: write the comment-rich default template as
        // bytes. Deliberately NOT a round-trip through a config struct —
        // the template is documentation, and serializing a struct would
        // strip every comment it ships with.
        jinn_preferences_config::create_default_preferences_to(prefs_path)
            .change_context(InstallError)
            .attach("failed to create jinn.toml with the default template")?;
        JinnTomlOutcome::Created(prefs_path.to_owned())
    };

    // The stream-rules file follows the same exactly-once contract as
    // `jinn.toml`: written when missing, never read or modified when
    // present — including under `overwrite`. The sibling location is the
    // same directory the preferences file lives in, which is what the
    // runtime read path resolves against.
    // `parent()` is `None` only for the filesystem root, which is then its
    // own parent for the sibling lookup.
    let prefs_parent = prefs_path.parent().map_or_else(
        || prefs_path.clone(),
        |p| {
            MarkedPathBuf::<Absolute>::new(p.as_path())
                .expect("parent of an absolute path is absolute")
        },
    );
    let stream_rules_path = prefs_parent
        .join(jinn_preferences_config::STREAM_RULES_FILE_NAME)
        .change_context(InstallError)
        .attach("failed to build stream_rules.toml path")?;
    let stream_rules = jinn_preferences_config::init_stream_rules_for_install(&stream_rules_path)
        .change_context(InstallError)
        .attach("failed to create stream_rules.toml with the embedded template")?;

    Ok(InstallReport {
        outcomes,
        jinn_toml,
        stream_rules,
    })
}

/// Installs a single bundled resource, returning its outcome.
///
/// The payload is written verbatim from its embedded bytes; whether it is valid
/// UTF-8 is irrelevant to installation.
fn install_resource(
    resource: &Bundled,
    destinations: &Destinations,
    overwrite: bool,
) -> Result<InstallOutcome, Report<InstallError>> {
    let destination = resource
        .destination(destinations)
        .change_context(InstallError)
        .attach("failed to build destination path")?;
    let existed = destination.exists();

    if existed && !overwrite {
        return Ok(InstallOutcome::Skipped(destination));
    }

    write_resource(&destination.as_marked_path(), resource.contents())?;

    Ok(final_outcome(destination, existed))
}

/// Writes `bytes` to `destination`, creating parent directories first.
fn write_resource(
    destination: &MarkedPath<Absolute>,
    bytes: &[u8],
) -> Result<(), Report<InstallError>> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .change_context(InstallError)
            .attach("failed to create destination directory")
            .attach(format!("path: {}", parent.to_string_lossy()))?;
    }

    std::fs::write(destination, bytes)
        .change_context(InstallError)
        .attach("failed to write resource")
        .attach(format!("path: {}", destination.to_string_lossy()))
}

/// The outcome for a write that happened: overwritten vs created.
fn final_outcome(destination: MarkedPathBuf<Absolute>, existed: bool) -> InstallOutcome {
    if existed {
        InstallOutcome::Overwritten(destination)
    } else {
        InstallOutcome::Created(destination)
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unwrap_used,
        reason = "test code"
    )]

    use super::*;

    /// Builds a [`Destinations`] rooted at five distinct temp dirs. The
    /// returned temps must outlive the destinations.
    fn fresh_destinations() -> (Destinations, Vec<jinn_testutil::JinnTempDir>) {
        let themes = jinn_testutil::tempdir();
        let personas = jinn_testutil::tempdir();
        let prompts = jinn_testutil::tempdir();
        let workflows = jinn_testutil::tempdir();
        let skills = jinn_testutil::tempdir();
        let destinations = Destinations::new(
            themes.path.as_marked_path(),
            personas.path.as_marked_path(),
            prompts.path.as_marked_path(),
            workflows.path.as_marked_path(),
            skills.path.as_marked_path(),
        );
        let temps = vec![themes, personas, prompts, workflows, skills];
        (destinations, temps)
    }

    /// An install environment: fresh destinations plus a prefs file path that
    /// does **not** exist yet (inside its own temp dir), backed by the real
    /// filesystem storage so the auto-create/patch semantics the existence
    /// gate depends on are exercised end to end.
    struct TestEnv {
        destinations: Destinations,
        prefs_path: MarkedPathBuf<Absolute>,
        _temps: Vec<jinn_testutil::JinnTempDir>,
    }

    impl TestEnv {
        fn fresh() -> Self {
            let (destinations, mut temps) = fresh_destinations();
            let prefs_dir = jinn_testutil::tempdir();
            let prefs_path = prefs_dir
                .path
                .join("jinn.toml")
                .expect("temp root joins a bare name");
            temps.push(prefs_dir);
            Self {
                destinations,
                prefs_path,
                _temps: temps,
            }
        }

        fn run(&self, overwrite: bool) -> InstallReport {
            install_defaults_to(&self.destinations, overwrite, &self.prefs_path).expect("install")
        }
    }

    /// Locates the outcome for a specific resource relative path.
    fn outcome_for<'a>(outcomes: &'a [InstallOutcome], relative: &str) -> &'a InstallOutcome {
        outcomes
            .iter()
            .find(|o| o.path().ends_with(relative))
            .unwrap_or_else(|| panic!("no outcome ending in {relative}"))
    }

    #[rstest::rstest]
    #[test]
    fn install_creates_theme_when_absent() {
        // Given destinations with no existing themes.
        let env = TestEnv::fresh();

        // When installing defaults.
        let report = env.run(false);

        // Then the `default.toml` theme was created.
        let outcome = outcome_for(&report.outcomes, "default.toml");
        assert!(
            matches!(outcome, InstallOutcome::Created(_)),
            "default.toml should be Created"
        );
        // And the file exists with non-empty contents.
        let written = std::fs::read_to_string(outcome.path()).expect("read");
        assert_ne!(written, "");
    }

    #[rstest::rstest]
    #[test]
    fn install_skips_theme_when_present() {
        // Given a destinations dir where `default.toml` already exists.
        let env = TestEnv::fresh();
        let existing = env
            .destinations
            .themes()
            .join("default.toml")
            .expect("dest root joins a bare name");
        std::fs::create_dir_all(env.destinations.themes()).unwrap();
        std::fs::write(&existing, "PRE-EXISTING").unwrap();

        // When installing defaults.
        let report = env.run(false);

        // Then `default.toml` is skipped (not overwritten).
        let outcome = outcome_for(&report.outcomes, "default.toml");
        assert!(
            matches!(outcome, InstallOutcome::Skipped(_)),
            "default.toml should be Skipped"
        );
        // And the original contents are untouched.
        let contents = std::fs::read_to_string(&existing).expect("read");
        assert_eq!(contents, "PRE-EXISTING");
    }

    #[rstest::rstest]
    #[test]
    fn install_creates_parent_dirs_when_missing() {
        // Given destinations whose root dirs do not exist at all.
        let themes = jinn_testutil::tempdir();
        let personas = jinn_testutil::tempdir();
        let prompts = jinn_testutil::tempdir();
        let workflows = jinn_testutil::tempdir();
        let skills = jinn_testutil::tempdir();
        let prefs_dir = jinn_testutil::tempdir();
        // Non-existent subdirs under each temp root.
        let destinations = Destinations::new(
            themes
                .path
                .join("themes")
                .expect("dest root joins a bare name")
                .as_marked_path(),
            personas
                .path
                .join("personas")
                .expect("dest root joins a bare name")
                .as_marked_path(),
            prompts
                .path
                .join("prompts")
                .expect("dest root joins a bare name")
                .as_marked_path(),
            workflows
                .path
                .join("workflows")
                .expect("dest root joins a bare name")
                .as_marked_path(),
            skills
                .path
                .join("skills")
                .expect("dest root joins a bare name")
                .as_marked_path(),
        );
        let prefs_path = prefs_dir
            .path
            .join_all(["nested", "jinn.toml"])
            .expect("name joins keep the base absolute");

        // When installing defaults.
        let result = install_defaults_to(&destinations, false, &prefs_path);

        // Then it succeeds (parents created) rather than erroring.
        assert!(result.is_ok(), "install should create missing parents");
    }

    #[rstest::rstest]
    #[test]
    fn install_creates_persona() {
        // Given destinations with no existing personas.
        let env = TestEnv::fresh();

        // When installing defaults.
        let report = env.run(false);

        // Then the stable bundled persona lands under the personas root.
        // `coding-assistant.md` is one of the resources guaranteed to ship.
        let outcome = outcome_for(&report.outcomes, "coding-assistant.md");
        assert!(
            outcome.path().starts_with(env.destinations.personas()),
            "persona should be under the personas root"
        );
        assert!(
            matches!(outcome, InstallOutcome::Created(_)),
            "persona should be Created"
        );
    }

    #[rstest::rstest]
    #[test]
    fn install_preserves_skill_subdir() {
        // Given destinations with no existing skills.
        let env = TestEnv::fresh();

        // When installing defaults.
        let report = env.run(false);

        // Then the nested skill keeps its `<name>/SKILL.md` structure.
        let outcome = outcome_for(&report.outcomes, "simple-task-loop/SKILL.md");
        assert!(
            matches!(outcome, InstallOutcome::Created(_)),
            "skill should be Created"
        );
        // And the file exists at the nested path.
        assert!(outcome.path().is_file());
    }

    #[rstest::rstest]
    #[test]
    fn install_creates_micro_task_loop_skill() {
        // Given destinations with no existing skills.
        let env = TestEnv::fresh();

        // When installing defaults.
        let report = env.run(false);

        // Then the micro-task-loop skill was created under the skills root.
        let outcome = outcome_for(&report.outcomes, "micro-task-loop/SKILL.md");
        assert!(
            matches!(outcome, InstallOutcome::Created(_)),
            "skill should be Created"
        );
        // And the file exists at the nested path.
        assert!(outcome.path().is_file());
    }

    #[rstest::rstest]
    #[test]
    fn install_preserves_jinn_usage_skill_subdir() {
        // Given destinations with no existing skills.
        let env = TestEnv::fresh();

        // When installing defaults.
        let report = env.run(false);

        // Then the stable bundled skill keeps its `<name>/SKILL.md`
        // structure — `jinn-usage` is one of the resources guaranteed to
        // ship, nested under its own directory.
        let outcome = outcome_for(&report.outcomes, "jinn-usage/SKILL.md");
        assert!(
            outcome.path().starts_with(env.destinations.skills()),
            "skill should be under the skills root"
        );
        assert!(
            matches!(outcome, InstallOutcome::Created(_)),
            "skill should be Created"
        );
        // And the file exists at the nested path with non-empty contents.
        assert!(outcome.path().is_file());
        assert_ne!(std::fs::read_to_string(outcome.path()).expect("read"), "");
    }

    #[rstest::rstest]
    #[test]
    fn install_preserves_workflow_subdir() {
        // Given destinations with no existing workflows.
        let env = TestEnv::fresh();

        // When installing defaults.
        let report = env.run(false);

        // Then the stable bundled workflow keeps its directory structure —
        // `autodev` is one of the resources guaranteed to ship, nested under
        // its own directory.
        let outcome = outcome_for(&report.outcomes, "autodev/workflow.md");
        assert!(
            outcome.path().starts_with(env.destinations.workflows()),
            "workflow should be under the workflows root"
        );
        assert!(
            matches!(outcome, InstallOutcome::Created(_)),
            "workflow should be Created"
        );
        // And the file exists at the nested path.
        assert!(outcome.path().is_file());
    }

    #[rstest::rstest]
    #[test]
    fn install_is_idempotent() {
        // Given a fresh set of destinations.
        let env = TestEnv::fresh();

        // When running install a second time (after a first full run).
        env.run(false);
        let second = env.run(false);

        // Then every outcome is Skipped and nothing reports Created.
        assert!(
            second
                .outcomes
                .iter()
                .all(|o| matches!(o, InstallOutcome::Skipped(_))),
            "second run must skip everything"
        );
        // And jinn.toml is reported untouched on the second run.
        assert!(
            matches!(second.jinn_toml, JinnTomlOutcome::Untouched(_)),
            "second run must not rewrite jinn.toml"
        );
    }

    #[rstest::rstest]
    #[test]
    fn install_outcomes_include_full_paths() {
        // Given a fresh set of destinations.
        let env = TestEnv::fresh();

        // When installing defaults.
        let report = env.run(false);

        // Then every outcome path is absolute (full path for the CLI to print).
        assert!(
            report
                .outcomes
                .iter()
                .all(|o| o.path().as_path().is_absolute()),
            "every outcome must carry an absolute path"
        );
        // And the install covered at least one bundled resource.
        assert_ne!(report.outcomes, [] as [InstallOutcome; 0]);
    }

    #[rstest::rstest]
    #[test]
    fn install_overwrites_theme_when_force() {
        // Given a destinations dir where `default.toml` already exists.
        let env = TestEnv::fresh();
        let existing = env
            .destinations
            .themes()
            .join("default.toml")
            .expect("dest root joins a bare name");
        std::fs::create_dir_all(env.destinations.themes()).unwrap();
        std::fs::write(&existing, "PRE-EXISTING").unwrap();

        // When installing defaults with overwrite enabled.
        let report = env.run(true);

        // Then `default.toml` is reported as overwritten (not skipped).
        let outcome = outcome_for(&report.outcomes, "default.toml");
        assert!(
            matches!(outcome, InstallOutcome::Overwritten(_)),
            "default.toml should be Overwritten"
        );
    }

    #[rstest::rstest]
    #[test]
    fn install_force_replaces_with_bundled_contents() {
        // Given a destinations dir where `default.toml` holds stale contents,
        // and a second destinations dir installed fresh to capture the bundled bytes.
        let env = TestEnv::fresh();
        let bundled_env = TestEnv::fresh();
        let existing = env
            .destinations
            .themes()
            .join("default.toml")
            .expect("dest root joins a bare name");
        std::fs::create_dir_all(env.destinations.themes()).unwrap();
        std::fs::write(&existing, "PRE-EXISTING").unwrap();
        bundled_env.run(false);
        let bundled_default = bundled_env
            .destinations
            .themes()
            .join("default.toml")
            .expect("dest root joins a bare name");
        let expected = std::fs::read_to_string(bundled_default.as_path()).expect("read bundled");

        // When installing with overwrite enabled.
        env.run(true);

        // Then the overwritten file matches the bundled contents, not the stale value.
        let contents = std::fs::read_to_string(&existing).expect("read");
        assert_eq!(contents, expected);
    }

    #[rstest::rstest]
    #[test]
    fn install_idempotent_under_force() {
        // Given a fully-installed destinations dir (files already match the bundled bytes).
        let env = TestEnv::fresh();
        env.run(false);

        // When installing again with overwrite enabled.
        let second = env.run(true);

        // Then every outcome is Overwritten — overwrite rewrites unconditionally,
        // with no content-diff short-circuit that would report Skipped.
        assert!(
            second
                .outcomes
                .iter()
                .all(|o| matches!(o, InstallOutcome::Overwritten(_))),
            "force run must overwrite everything, even unchanged files"
        );
        // And jinn.toml remains untouched even under force.
        assert!(
            matches!(second.jinn_toml, JinnTomlOutcome::Untouched(_)),
            "force must never rewrite an existing jinn.toml"
        );
    }

    #[rstest::rstest]
    #[test]
    fn install_creates_jinn_toml_once_on_fresh_env() {
        // Given a fresh environment with no jinn.toml.
        let env = TestEnv::fresh();
        assert!(!env.prefs_path.exists());

        // When installing defaults.
        let report = env.run(false);

        // Then jinn.toml is reported Created.
        assert_eq!(
            report.jinn_toml,
            JinnTomlOutcome::Created(env.prefs_path.clone())
        );
        // And the file exists on disk.
        assert!(env.prefs_path.exists(), "jinn.toml must be created on disk");
    }

    #[rstest::rstest]
    #[test]
    fn install_reports_untouched_jinn_toml_when_file_exists() {
        // Given an environment where jinn.toml already exists (even malformed).
        let env = TestEnv::fresh();
        if let Some(parent) = env.prefs_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&env.prefs_path, "NOT [valid toml").unwrap();

        // When installing defaults.
        let report = env.run(false);

        // Then jinn.toml is reported Untouched.
        assert_eq!(
            report.jinn_toml,
            JinnTomlOutcome::Untouched(env.prefs_path.clone())
        );
        // And the malformed file is left byte-identical — which is itself the
        // proof that nothing was registered (a reload-based assertion is
        // impossible: the file does not parse).
        let on_disk = std::fs::read_to_string(&env.prefs_path).expect("read");
        assert_eq!(on_disk, "NOT [valid toml");
    }

    #[rstest::rstest]
    #[test]
    fn install_force_leaves_existing_jinn_toml_byte_identical() {
        // Given an environment where jinn.toml exists with a user customization
        // and a theme file already exists on disk.
        let env = TestEnv::fresh();
        if let Some(parent) = env.prefs_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let original = "# my edits\n[compaction]\nthreshold = 0.4\n";
        std::fs::write(&env.prefs_path, original).unwrap();
        let existing = env
            .destinations
            .themes()
            .join("default.toml")
            .expect("dest root joins a bare name");
        std::fs::create_dir_all(env.destinations.themes()).unwrap();
        std::fs::write(&existing, "PRE-EXISTING").unwrap();

        // When installing defaults with force.
        let report = env.run(true);

        // Then jinn.toml is Untouched and byte-identical on disk.
        assert_eq!(
            report.jinn_toml,
            JinnTomlOutcome::Untouched(env.prefs_path.clone())
        );
        let on_disk = std::fs::read_to_string(&env.prefs_path).expect("read");
        assert_eq!(on_disk, original);
        // And resource files were still overwritten.
        let theme_outcome = outcome_for(&report.outcomes, "default.toml");
        assert!(
            matches!(theme_outcome, InstallOutcome::Overwritten(_)),
            "resource files must still follow --force"
        );
    }

    #[rstest::rstest]
    #[test]
    fn install_creates_the_stream_rules_file_on_a_fresh_env() {
        // Given a fresh environment with no stream_rules.toml.
        let env = TestEnv::fresh();

        // When installing defaults.
        let report = env.run(false);

        // Then the rules file is created beside jinn.toml with the embedded
        // template.
        let expected = env
            .prefs_path
            .parent()
            .unwrap_or(env.prefs_path.as_marked_path())
            .join(jinn_preferences_config::STREAM_RULES_FILE_NAME)
            .expect("typed parent joins a bare name");
        assert_eq!(
            report.stream_rules,
            StreamRulesSeedOutcome::Created(expected.clone())
        );
        let on_disk = std::fs::read_to_string(&expected).expect("read rules file");
        assert_eq!(
            on_disk,
            jinn_preferences_config::STREAM_RULES_TEMPLATE,
            "the created file is the embedded template, verbatim"
        );
    }

    #[rstest::rstest]
    #[test]
    fn install_never_touches_an_existing_stream_rules_file() {
        // Given an environment where stream_rules.toml exists with user
        // content (malformed on purpose: it must never even be read).
        let env = TestEnv::fresh();
        let rules_path = env
            .prefs_path
            .parent()
            .unwrap_or(env.prefs_path.as_marked_path())
            .join(jinn_preferences_config::STREAM_RULES_FILE_NAME)
            .expect("typed parent joins a bare name");
        std::fs::create_dir_all(rules_path.parent().unwrap()).unwrap();
        let original = "# my rules\nNOT [valid toml\n";
        std::fs::write(&rules_path, original).unwrap();

        // When installing defaults.
        let report = env.run(false);

        // Then it is reported untouched.
        assert_eq!(
            report.stream_rules,
            StreamRulesSeedOutcome::Untouched(rules_path.clone())
        );
        // And the file is byte-identical — the proof nothing read or
        // rewrote it.
        assert_eq!(
            std::fs::read_to_string(&rules_path).expect("read"),
            original
        );
    }

    #[rstest::rstest]
    #[test]
    fn install_force_leaves_an_existing_stream_rules_file_byte_identical() {
        // Given an installed environment whose rules file carries user edits.
        let env = TestEnv::fresh();
        env.run(false);
        let rules_path = env
            .prefs_path
            .parent()
            .unwrap_or(env.prefs_path.as_marked_path())
            .join(jinn_preferences_config::STREAM_RULES_FILE_NAME)
            .expect("typed parent joins a bare name");
        let edited = "# my edits\n[[stream_rules.entry]]\nname = 'mine'\nconditions = ['X']\nscopes = ['tool:bash']\nbody = 'x'\n";
        std::fs::write(&rules_path, edited).unwrap();
        let existing = env
            .destinations
            .themes()
            .join("default.toml")
            .expect("dest root joins a bare name");
        std::fs::create_dir_all(env.destinations.themes()).unwrap();
        std::fs::write(&existing, "PRE-EXISTING").unwrap();

        // When installing with force.
        let report = env.run(true);

        // Then resource files were overwritten but the rules file is
        // untouched and byte-identical: force does not apply to it, the
        // same contract as jinn.toml.
        let theme_outcome = outcome_for(&report.outcomes, "default.toml");
        assert!(matches!(theme_outcome, InstallOutcome::Overwritten(_)));
        assert_eq!(
            report.stream_rules,
            StreamRulesSeedOutcome::Untouched(rules_path.clone())
        );
        assert_eq!(std::fs::read_to_string(&rules_path).expect("read"), edited);
    }

    /// A nested skill reference (a file beneath a skill's own directory)
    /// installs with its structure intact, at the same relative path under the
    /// skills root.
    #[rstest::rstest]
    #[test]
    fn install_preserves_nested_skill_reference_path() {
        // Given a fresh set of destinations.
        let env = TestEnv::fresh();

        // When installing defaults.
        let report = env.run(false);

        // Then some reference nested beneath a skill directory lands under the
        // skills root at its own nested relative path.
        let nested = report
            .outcomes
            .iter()
            .map(InstallOutcome::path)
            .find(|path| {
                path.starts_with(env.destinations.skills())
                    && path
                        .strip_prefix(env.destinations.skills())
                        .is_ok_and(|rest| rest.components().count() > 2)
            })
            .expect("a skill reference nested under skills/<name>/");
        assert!(nested.is_file());
    }

    /// Every installed file is non-empty: the embedding carries the whole
    /// bundled catalogue, but the tests do not read `res/` — the repo's
    /// resource tree is content, not test fixture data. Existence coverage
    /// for specific stable resources lives in the per-kind tests above.
    #[rstest::rstest]
    #[test]
    fn installed_payloads_are_non_empty() {
        // Given a fresh set of destinations.
        let env = TestEnv::fresh();

        // When installing defaults.
        let report = env.run(false);

        // Then every installed file exists with bytes in it.
        assert!(!report.outcomes.is_empty(), "the catalogue is not empty");
        for outcome in &report.outcomes {
            let installed = outcome.path();
            let bytes =
                std::fs::read(installed).unwrap_or_else(|e| panic!("{installed} reads: {e}"));
            assert!(!bytes.is_empty(), "{installed} installed empty");
        }
    }
}
