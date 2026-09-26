//! Default resource installation — seeds themes, personas, prompts, and
//! skills into the user's config and agent directories.
//!
//! Every resource under `res/` is embedded at compile time (`include_str!`),
//! so the binary is self-contained.
//!
//! [`install_defaults_to`] (`jinn install`) is a pure seeder. Payload files
//! follow skip/`--force` rules; `jinn.toml` is written **exactly once**,
//! only when it does not exist. An existing file is never read or modified —
//! even with `--force` — so user edits survive and a malformed file never
//! fails the install.

use std::path::{Path, PathBuf};

use error_stack::{Report, ResultExt};
use wherror::Error;

/// Relative destinations for the four resource kinds.
///
/// Each field is a root directory: themes/personas/prompts live under the
/// config dir (`~/.config/jinn`), skills live under the agent dir
/// (`~/.agents/skills`). Passed by value into [`install_defaults_to`] so
/// tests can point at temp dirs.
#[derive(Debug, Clone)]
pub struct Destinations {
    themes: PathBuf,
    personas: PathBuf,
    prompts: PathBuf,
    skills: PathBuf,
}

impl Destinations {
    /// Creates a destination set from the four root directories.
    #[must_use]
    pub fn new(themes: PathBuf, personas: PathBuf, prompts: PathBuf, skills: PathBuf) -> Self {
        Self {
            themes,
            personas,
            prompts,
            skills,
        }
    }
}

/// Where a bundled resource should be installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Theme,
    Persona,
    Prompt,
    Skill,
}

impl Kind {
    /// Resolves this kind to its destination root within `destinations`.
    fn root(self, destinations: &Destinations) -> &Path {
        match self {
            Kind::Theme => &destinations.themes,
            Kind::Persona => &destinations.personas,
            Kind::Prompt => &destinations.prompts,
            Kind::Skill => &destinations.skills,
        }
    }
}

/// One bundled resource: its kind, its relative path under its destination
/// root, and its compile-time-embedded contents.
struct Bundled {
    kind: Kind,
    /// Path relative to the destination root (e.g. `default.toml`,
    /// `phased-task-loop/SKILL.md`).
    relative: &'static str,
    /// The embedded payload, written verbatim.
    contents: &'static str,
}

/// Outcome of installing one resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallOutcome {
    /// Resource was written to a previously-missing path.
    Created(PathBuf),
    /// Resource already existed and was left untouched.
    Skipped(PathBuf),
    /// Resource already existed and was overwritten in place.
    Overwritten(PathBuf),
}

impl InstallOutcome {
    /// The full destination path this outcome refers to.
    #[must_use]
    pub fn path(&self) -> &Path {
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
    /// Per-resource outcomes in [`BUNDLED`] order (deterministic).
    pub outcomes: Vec<InstallOutcome>,
    /// Outcome for the user preferences file itself.
    pub jinn_toml: JinnTomlOutcome,
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
    Created(PathBuf),
    /// The file already existed and was never read or modified this run.
    Untouched(PathBuf),
}

/// The compile-time catalogue of bundled defaults.
///
/// Ordering is deterministic; outcomes are returned in this order.
const BUNDLED: &[Bundled] = &[
    // --- themes ---
    Bundled {
        kind: Kind::Theme,
        relative: "catppuccin-mocha.toml",
        contents: include_str!("../../../../../res/themes/catppuccin-mocha.toml"),
    },
    Bundled {
        kind: Kind::Theme,
        relative: "default.toml",
        contents: include_str!("../../../../../res/themes/default.toml"),
    },
    Bundled {
        kind: Kind::Theme,
        relative: "nord-light.toml",
        contents: include_str!("../../../../../res/themes/nord-light.toml"),
    },
    Bundled {
        kind: Kind::Theme,
        relative: "gruvbox-dark.toml",
        contents: include_str!("../../../../../res/themes/gruvbox-dark.toml"),
    },
    Bundled {
        kind: Kind::Theme,
        relative: "sonokai.toml",
        contents: include_str!("../../../../../res/themes/sonokai.toml"),
    },
    // --- personas ---
    Bundled {
        kind: Kind::Persona,
        relative: "brainstorm.md",
        contents: include_str!("../../../../../res/personas/brainstorm.md"),
    },
    Bundled {
        kind: Kind::Persona,
        relative: "coding-assistant.md",
        contents: include_str!("../../../../../res/personas/coding-assistant.md"),
    },
    Bundled {
        kind: Kind::Persona,
        relative: "general.md",
        contents: include_str!("../../../../../res/personas/general.md"),
    },
    Bundled {
        kind: Kind::Persona,
        relative: "learning-tutor.md",
        contents: include_str!("../../../../../res/personas/learning-tutor.md"),
    },
    // --- prompts ---
    Bundled {
        kind: Kind::Prompt,
        relative: "approve-plan.md",
        contents: include_str!("../../../../../res/prompts/approve-plan.md"),
    },
    Bundled {
        kind: Kind::Prompt,
        relative: "_compaction.md",
        contents: include_str!("../../../../../res/prompts/_compaction.md"),
    },
    Bundled {
        kind: Kind::Prompt,
        relative: "gap-analysis.md",
        contents: include_str!("../../../../../res/prompts/gap-analysis.md"),
    },
    Bundled {
        kind: Kind::Prompt,
        relative: "generate-persona.md",
        contents: include_str!("../../../../../res/prompts/generate-persona.md"),
    },
    Bundled {
        kind: Kind::Prompt,
        relative: "meta-prompt.md",
        contents: include_str!("../../../../../res/prompts/meta-prompt.md"),
    },
    Bundled {
        kind: Kind::Prompt,
        relative: "plan.md",
        contents: include_str!("../../../../../res/prompts/plan.md"),
    },
    Bundled {
        kind: Kind::Prompt,
        relative: "research.md",
        contents: include_str!("../../../../../res/prompts/research.md"),
    },
    // --- skills (preserve nested subdir structure) ---
    Bundled {
        kind: Kind::Skill,
        relative: "phased-task-loop/SKILL.md",
        contents: include_str!("../../../../../res/skills/phased-task-loop/SKILL.md"),
    },
    Bundled {
        kind: Kind::Skill,
        relative: "simple-task-loop/SKILL.md",
        contents: include_str!("../../../../../res/skills/simple-task-loop/SKILL.md"),
    },
    Bundled {
        kind: Kind::Skill,
        relative: "micro-task-loop/SKILL.md",
        contents: include_str!("../../../../../res/skills/micro-task-loop/SKILL.md"),
    },
    Bundled {
        kind: Kind::Skill,
        relative: "jinn-usage/SKILL.md",
        contents: include_str!("../../../../../res/skills/jinn-usage/SKILL.md"),
    },
    Bundled {
        kind: Kind::Skill,
        relative: "jinn-usage/references/keybindings.md",
        contents: include_str!("../../../../../res/skills/jinn-usage/references/keybindings.md"),
    },
    Bundled {
        kind: Kind::Skill,
        relative: "jinn-usage/references/context-management.md",
        contents: include_str!(
            "../../../../../res/skills/jinn-usage/references/context-management.md"
        ),
    },
    Bundled {
        kind: Kind::Skill,
        relative: "jinn-usage/references/sessions-and-subagents.md",
        contents: include_str!(
            "../../../../../res/skills/jinn-usage/references/sessions-and-subagents.md"
        ),
    },
    Bundled {
        kind: Kind::Skill,
        relative: "jinn-usage/references/pickers-and-search.md",
        contents: include_str!(
            "../../../../../res/skills/jinn-usage/references/pickers-and-search.md"
        ),
    },
    Bundled {
        kind: Kind::Skill,
        relative: "jinn-usage/references/terminal-overlay.md",
        contents: include_str!(
            "../../../../../res/skills/jinn-usage/references/terminal-overlay.md"
        ),
    },
    Bundled {
        kind: Kind::Skill,
        relative: "jinn-usage/references/mcp-servers.md",
        contents: include_str!("../../../../../res/skills/jinn-usage/references/mcp-servers.md"),
    },
    Bundled {
        kind: Kind::Skill,
        relative: "jinn-usage/references/chat-input-tokens.md",
        contents: include_str!(
            "../../../../../res/skills/jinn-usage/references/chat-input-tokens.md"
        ),
    },
    Bundled {
        kind: Kind::Skill,
        relative: "jinn-usage/references/models-and-providers.md",
        contents: include_str!(
            "../../../../../res/skills/jinn-usage/references/models-and-providers.md"
        ),
    },
    Bundled {
        kind: Kind::Skill,
        relative: "jinn-usage/references/configuration.md",
        contents: include_str!("../../../../../res/skills/jinn-usage/references/configuration.md"),
    },
];

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
/// Outcomes are returned in [`BUNDLED`] order (deterministic), alongside the
/// `jinn.toml` outcome.
///
/// # Errors
///
/// Returns [`Report<InstallError>`] if directory creation or file writing
/// fails, or if preferences cannot be created on the fresh-create path.
pub fn install_defaults_to(
    destinations: &Destinations,
    overwrite: bool,
    prefs_path: &Path,
) -> Result<InstallReport, Report<InstallError>> {
    let prefs_existed = prefs_path.exists();

    let outcomes: Vec<InstallOutcome> = BUNDLED
        .iter()
        .map(|resource| install_text(resource, resource.contents, destinations, overwrite))
        .collect::<Result<Vec<_>, Report<InstallError>>>()?;

    let jinn_toml = if prefs_existed {
        JinnTomlOutcome::Untouched(prefs_path.to_path_buf())
    } else {
        // Fresh-create path: write the comment-rich default template as
        // bytes. Deliberately NOT a round-trip through a config struct —
        // the template is documentation, and serializing a struct would
        // strip every comment it ships with.
        jinn_preferences_config::create_default_preferences_to(prefs_path)
            .change_context(InstallError)
            .attach("failed to create jinn.toml with the default template")?;
        JinnTomlOutcome::Created(prefs_path.to_path_buf())
    };

    Ok(InstallReport {
        outcomes,
        jinn_toml,
    })
}

/// Installs a single bundled text resource, returning its outcome.
fn install_text(
    resource: &Bundled,
    contents: &str,
    destinations: &Destinations,
    overwrite: bool,
) -> Result<InstallOutcome, Report<InstallError>> {
    let destination = resource.kind.root(destinations).join(resource.relative);
    let existed = destination.exists();

    if existed && !overwrite {
        return Ok(InstallOutcome::Skipped(destination));
    }

    write_resource(&destination, contents.as_bytes())?;

    Ok(final_outcome(destination, existed))
}

/// Writes `bytes` to `destination`, creating parent directories first.
fn write_resource(destination: &Path, bytes: &[u8]) -> Result<(), Report<InstallError>> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .change_context(InstallError)
            .attach("failed to create destination directory")
            .attach(format!("path: {}", parent.display()))?;
    }

    std::fs::write(destination, bytes)
        .change_context(InstallError)
        .attach("failed to write resource")
        .attach(format!("path: {}", destination.display()))
}

/// The outcome for a write that happened: overwritten vs created.
fn final_outcome(destination: PathBuf, existed: bool) -> InstallOutcome {
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
    use tempfile::TempDir;

    /// Builds a [`Destinations`] rooted at four distinct temp dirs. The
    /// returned temps must outlive the destinations.
    fn fresh_destinations() -> (Destinations, Vec<TempDir>) {
        let themes = TempDir::new().unwrap();
        let personas = TempDir::new().unwrap();
        let prompts = TempDir::new().unwrap();
        let skills = TempDir::new().unwrap();
        let destinations = Destinations::new(
            themes.path().to_path_buf(),
            personas.path().to_path_buf(),
            prompts.path().to_path_buf(),
            skills.path().to_path_buf(),
        );
        let temps = vec![themes, personas, prompts, skills];
        (destinations, temps)
    }

    /// An install environment: fresh destinations plus a prefs file path that
    /// does **not** exist yet (inside its own temp dir), backed by the real
    /// filesystem storage so the auto-create/patch semantics the existence
    /// gate depends on are exercised end to end.
    struct TestEnv {
        destinations: Destinations,
        prefs_path: std::path::PathBuf,
        _temps: Vec<TempDir>,
    }

    impl TestEnv {
        fn fresh() -> Self {
            let (destinations, mut temps) = fresh_destinations();
            let prefs_dir = TempDir::new().unwrap();
            let prefs_path = prefs_dir.path().join("jinn.toml");
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
        assert!(!written.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn install_skips_theme_when_present() {
        // Given a destinations dir where `default.toml` already exists.
        let env = TestEnv::fresh();
        let existing = env.destinations.themes.join("default.toml");
        std::fs::create_dir_all(env.destinations.themes.clone()).unwrap();
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
        let themes = TempDir::new().unwrap();
        let personas = TempDir::new().unwrap();
        let prompts = TempDir::new().unwrap();
        let skills = TempDir::new().unwrap();
        let prefs_dir = TempDir::new().unwrap();
        // Non-existent subdirs under each temp root.
        let destinations = Destinations::new(
            themes.path().join("themes"),
            personas.path().join("personas"),
            prompts.path().join("prompts"),
            skills.path().join("skills"),
        );
        let prefs_path = prefs_dir.path().join("nested").join("jinn.toml");

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

        // Then `general.md` lands under the personas root.
        let outcome = outcome_for(&report.outcomes, "general.md");
        assert!(
            outcome.path().starts_with(&env.destinations.personas),
            "persona should be under the personas root"
        );
        assert!(
            matches!(outcome, InstallOutcome::Created(_)),
            "persona should be Created"
        );
    }

    #[rstest::rstest]
    #[test]
    fn install_creates_prompt() {
        // Given destinations with no existing prompts.
        let env = TestEnv::fresh();

        // When installing defaults.
        let report = env.run(false);

        // Then `plan.md` lands under the prompts root.
        let outcome = outcome_for(&report.outcomes, "plan.md");
        assert!(
            outcome.path().starts_with(&env.destinations.prompts),
            "prompt should be under the prompts root"
        );
    }

    #[rstest::rstest]
    #[test]
    fn install_creates_research_prompt() {
        // Given destinations with no existing prompts.
        let env = TestEnv::fresh();

        // When installing defaults.
        let report = env.run(false);

        // Then `research.md` lands under the prompts root.
        let outcome = outcome_for(&report.outcomes, "research.md");
        assert!(
            outcome.path().starts_with(&env.destinations.prompts),
            "prompt should be under the prompts root"
        );
        // And it reports Created.
        assert!(
            matches!(outcome, InstallOutcome::Created(_)),
            "research.md should be Created"
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
        let outcome = outcome_for(&report.outcomes, "phased-task-loop/SKILL.md");
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
            outcome.path().starts_with(&env.destinations.skills),
            "skill should be under the skills root"
        );
        assert!(
            matches!(outcome, InstallOutcome::Created(_)),
            "skill should be Created"
        );
        // And the file exists at the nested path with non-empty contents.
        assert!(outcome.path().is_file());
        assert!(
            !std::fs::read_to_string(outcome.path())
                .expect("read")
                .is_empty()
        );
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
            report.outcomes.iter().all(|o| o.path().is_absolute()),
            "every outcome must carry an absolute path"
        );
        // And the count matches the bundled catalogue size.
        assert_eq!(report.outcomes.len(), BUNDLED.len());
    }

    #[rstest::rstest]
    #[test]
    fn install_overwrites_theme_when_force() {
        // Given a destinations dir where `default.toml` already exists.
        let env = TestEnv::fresh();
        let existing = env.destinations.themes.join("default.toml");
        std::fs::create_dir_all(env.destinations.themes.clone()).unwrap();
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
        let existing = env.destinations.themes.join("default.toml");
        std::fs::create_dir_all(env.destinations.themes.clone()).unwrap();
        std::fs::write(&existing, "PRE-EXISTING").unwrap();
        bundled_env.run(false);
        let bundled_default = bundled_env.destinations.themes.join("default.toml");
        let expected = std::fs::read_to_string(&bundled_default).expect("read bundled");

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
        let existing = env.destinations.themes.join("default.toml");
        std::fs::create_dir_all(env.destinations.themes.clone()).unwrap();
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
    fn jinn_usage_router_links_resolve_to_bundled_references() {
        // Given the bundled jinn-usage SKILL.md body.
        let skill = BUNDLED
            .iter()
            .find(|b| b.kind == Kind::Skill && b.relative == "jinn-usage/SKILL.md")
            .expect("jinn-usage SKILL.md must be registered in BUNDLED");
        let body = skill.contents;

        // When extracting its `references/*.md` links.
        let linked: Vec<&str> = body
            .lines()
            .filter_map(|line| line.split("references/").nth(1))
            .filter_map(|rest| {
                let end = rest.find('`')?;
                let name = rest.get(..end)?;
                name.to_ascii_lowercase().ends_with(".md").then_some(name)
            })
            .collect();

        // Then the router links at least the core references.
        assert!(
            linked.len() >= 8,
            "expected the router to link its reference files, found {linked:?}"
        );
        // And every linked reference is bundled.
        for name in linked {
            let relative = format!("jinn-usage/references/{name}");
            assert!(
                BUNDLED
                    .iter()
                    .any(|b| b.kind == Kind::Skill && b.relative == relative),
                "SKILL.md links references/{name} but it is not registered in BUNDLED"
            );
        }
    }

    /// Every file under `res/skills/jinn-usage/` has a `BUNDLED` entry. A
    /// reference added on disk but not registered silently fails to install
    /// (`include_str!` only catches the opposite direction).
    #[rstest::rstest]
    #[test]
    fn every_jinn_usage_disk_file_is_registered_in_bundled() {
        // Given the on-disk jinn-usage skill directory.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../res/skills/jinn-usage");
        let mut disk_files: Vec<String> = Vec::new();
        if dir.join("SKILL.md").is_file() {
            disk_files.push("jinn-usage/SKILL.md".to_owned());
        }
        let refs = dir.join("references");
        if refs.is_dir() {
            for file in std::fs::read_dir(&refs).expect("read references") {
                let file = file.expect("read reference");
                let name = file.file_name().to_string_lossy().to_string();
                if name.to_ascii_lowercase().ends_with(".md") {
                    disk_files.push(format!("jinn-usage/references/{name}"));
                }
            }
        }

        // When comparing against the catalogue.
        // Then every disk file is registered.
        for relative in &disk_files {
            assert!(
                BUNDLED
                    .iter()
                    .any(|b| b.kind == Kind::Skill && b.relative == relative),
                "{relative} exists on disk but is not registered in BUNDLED"
            );
        }
        // And the directory holds the expected set (router + 9 references).
        assert_eq!(
            disk_files.len(),
            10,
            "unexpected file count under res/skills/jinn-usage: {disk_files:?}"
        );
    }

    /// The installed jinn-usage skill parses through the real skill scanner:
    /// valid frontmatter, a name matching its directory, and a
    /// non-empty description (the trigger surface agents match on).
    #[rstest::rstest]
    #[test]
    fn jinn_usage_installs_as_a_discoverable_skill() {
        // Given a fresh install.
        let env = TestEnv::fresh();
        env.run(false);

        // When scanning the skills destination.
        let skills = jinn_skills::scan_skills(&env.destinations.skills);

        // Then jinn-usage is discovered by name with a usable description.
        let usage = skills
            .iter()
            .find(|s| s.name == "jinn-usage")
            .expect("jinn-usage must be discovered by the skill scanner");
        assert!(!usage.description.trim().is_empty());
        // And its base_dir holds the router's reference files.
        assert!(usage.base_dir.join("references").is_dir());
    }
}
