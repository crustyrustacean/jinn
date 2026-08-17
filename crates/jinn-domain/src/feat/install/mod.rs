//! Default resource installation — seeds themes, personas, prompts, and skills
//! into the user's config and agent directories.
//!
//! Every resource under `res/` is embedded at compile time via
//! [`include_str!`], so the binary is self-contained.
//!
//! The pure [`install_defaults_to`] function performs the work and is fully
//! testable against [`tempfile::TempDir`] roots. The CLI prints the outcomes.

use std::path::{Path, PathBuf};

use error_stack::{Report, ResultExt};
use wherror::Error;

/// Relative destinations for the four resource kinds.
///
/// Each field is a root directory: themes/personas/prompts live under the
/// config dir (`~/.config/jinn`), skills live under the agent dir
/// (`~/.agents/skills`). Passed by value into [`install_defaults_to`] so tests
/// can point at temp dirs.
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
#[derive(Debug, Clone, Copy)]
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
/// Outcomes are returned in [`BUNDLED`] order (deterministic).
///
/// # Errors
///
/// Returns [`Report<InstallError>`] if directory creation or file writing fails.
pub fn install_defaults_to(
    destinations: &Destinations,
    overwrite: bool,
) -> Result<Vec<InstallOutcome>, Report<InstallError>> {
    BUNDLED
        .iter()
        .map(|resource| install_one(resource, destinations, overwrite))
        .collect()
}

/// Installs a single bundled resource, returning its outcome.
fn install_one(
    resource: &Bundled,
    destinations: &Destinations,
    overwrite: bool,
) -> Result<InstallOutcome, Report<InstallError>> {
    let destination = resource.kind.root(destinations).join(resource.relative);
    let existed = destination.exists();

    if existed && !overwrite {
        return Ok(InstallOutcome::Skipped(destination));
    }

    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .change_context(InstallError)
            .attach("failed to create destination directory")
            .attach(format!("path: {}", parent.display()))?;
    }

    std::fs::write(&destination, resource.contents)
        .change_context(InstallError)
        .attach("failed to write resource")
        .attach(format!("path: {}", destination.display()))?;

    if existed {
        Ok(InstallOutcome::Overwritten(destination))
    } else {
        Ok(InstallOutcome::Created(destination))
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

    /// Builds a [`Destinations`] rooted at four distinct temp dirs and returns
    /// them alongside the temps (which must outlive the destinations).
    fn fresh_destinations() -> (Destinations, [TempDir; 4]) {
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
        (destinations, [themes, personas, prompts, skills])
    }

    /// Locates the outcome for a specific resource relative path.
    fn outcome_for<'a>(outcomes: &'a [InstallOutcome], relative: &str) -> &'a InstallOutcome {
        outcomes
            .iter()
            .find(|o| o.path().ends_with(relative))
            .unwrap_or_else(|| panic!("no outcome ending in {relative}"))
    }

    #[test]
    fn install_creates_theme_when_absent() {
        // Given destinations with no existing themes.
        let (destinations, _temps) = fresh_destinations();

        // When installing defaults.
        let outcomes = install_defaults_to(&destinations, false).expect("install");

        // Then the `default.toml` theme was created.
        let outcome = outcome_for(&outcomes, "default.toml");
        assert!(
            matches!(outcome, InstallOutcome::Created(_)),
            "default.toml should be Created"
        );
        // And the file exists with non-empty contents.
        let written = std::fs::read_to_string(outcome.path()).expect("read");
        assert!(!written.is_empty());
    }

    #[test]
    fn install_skips_theme_when_present() {
        // Given a destinations dir where `default.toml` already exists.
        let (destinations, _temps) = fresh_destinations();
        let existing = destinations.themes.join("default.toml");
        std::fs::create_dir_all(destinations.themes.clone()).unwrap();
        std::fs::write(&existing, "PRE-EXISTING").unwrap();

        // When installing defaults.
        let outcomes = install_defaults_to(&destinations, false).expect("install");

        // Then `default.toml` is skipped (not overwritten).
        let outcome = outcome_for(&outcomes, "default.toml");
        assert!(
            matches!(outcome, InstallOutcome::Skipped(_)),
            "default.toml should be Skipped"
        );
        // And the original contents are untouched.
        let contents = std::fs::read_to_string(&existing).expect("read");
        assert_eq!(contents, "PRE-EXISTING");
    }

    #[test]
    fn install_creates_parent_dirs_when_missing() {
        // Given destinations whose root dirs do not exist at all.
        let themes = TempDir::new().unwrap();
        let personas = TempDir::new().unwrap();
        let prompts = TempDir::new().unwrap();
        let skills = TempDir::new().unwrap();
        // Non-existent subdirs under each temp root.
        let destinations = Destinations::new(
            themes.path().join("themes"),
            personas.path().join("personas"),
            prompts.path().join("prompts"),
            skills.path().join("skills"),
        );

        // When installing defaults.
        let result = install_defaults_to(&destinations, false);

        // Then it succeeds (parents created) rather than erroring.
        assert!(result.is_ok(), "install should create missing parents");
    }

    #[test]
    fn install_creates_persona() {
        // Given destinations with no existing personas.
        let (destinations, _temps) = fresh_destinations();

        // When installing defaults.
        let outcomes = install_defaults_to(&destinations, false).expect("install");

        // Then `general.md` lands under the personas root.
        let outcome = outcome_for(&outcomes, "general.md");
        assert!(
            outcome.path().starts_with(&destinations.personas),
            "persona should be under the personas root"
        );
        assert!(
            matches!(outcome, InstallOutcome::Created(_)),
            "persona should be Created"
        );
    }

    #[test]
    fn install_creates_prompt() {
        // Given destinations with no existing prompts.
        let (destinations, _temps) = fresh_destinations();

        // When installing defaults.
        let outcomes = install_defaults_to(&destinations, false).expect("install");

        // Then `plan.md` lands under the prompts root.
        let outcome = outcome_for(&outcomes, "plan.md");
        assert!(
            outcome.path().starts_with(&destinations.prompts),
            "prompt should be under the prompts root"
        );
    }

    #[test]
    fn install_preserves_skill_subdir() {
        // Given destinations with no existing skills.
        let (destinations, _temps) = fresh_destinations();

        // When installing defaults.
        let outcomes = install_defaults_to(&destinations, false).expect("install");

        // Then the nested skill keeps its `<name>/SKILL.md` structure.
        let outcome = outcome_for(&outcomes, "phased-task-loop/SKILL.md");
        assert!(
            matches!(outcome, InstallOutcome::Created(_)),
            "skill should be Created"
        );
        // And the file exists at the nested path.
        assert!(outcome.path().is_file());
    }

    #[test]
    fn install_is_idempotent() {
        // Given a fresh set of destinations.
        let (destinations, _temps) = fresh_destinations();

        // When running install a second time (after a first full run).
        install_defaults_to(&destinations, false).expect("first install");
        let second = install_defaults_to(&destinations, false).expect("second install");

        // Then every outcome is Skipped and nothing reports Created.
        assert!(
            second
                .iter()
                .all(|o| matches!(o, InstallOutcome::Skipped(_))),
            "second run must skip everything"
        );
    }

    #[test]
    fn install_outcomes_include_full_paths() {
        // Given a fresh set of destinations.
        let (destinations, _temps) = fresh_destinations();

        // When installing defaults.
        let outcomes = install_defaults_to(&destinations, false).expect("install");

        // Then every outcome path is absolute (full path for the CLI to print).
        assert!(
            outcomes.iter().all(|o| o.path().is_absolute()),
            "every outcome must carry an absolute path"
        );
        // And the count matches the bundled catalogue size.
        assert_eq!(outcomes.len(), BUNDLED.len());
    }

    #[test]
    fn install_overwrites_theme_when_force() {
        // Given a destinations dir where `default.toml` already exists.
        let (destinations, _temps) = fresh_destinations();
        let existing = destinations.themes.join("default.toml");
        std::fs::create_dir_all(destinations.themes.clone()).unwrap();
        std::fs::write(&existing, "PRE-EXISTING").unwrap();

        // When installing defaults with overwrite enabled.
        let outcomes = install_defaults_to(&destinations, true).expect("install");

        // Then `default.toml` is reported as overwritten (not skipped).
        let outcome = outcome_for(&outcomes, "default.toml");
        assert!(
            matches!(outcome, InstallOutcome::Overwritten(_)),
            "default.toml should be Overwritten"
        );
    }

    #[test]
    fn install_force_replaces_with_bundled_contents() {
        // Given a destinations dir where `default.toml` holds stale contents,
        // and a second destinations dir installed fresh to capture the bundled bytes.
        let (destinations, _temps) = fresh_destinations();
        let (bundled, _bundled_temps) = fresh_destinations();
        let existing = destinations.themes.join("default.toml");
        std::fs::create_dir_all(destinations.themes.clone()).unwrap();
        std::fs::write(&existing, "PRE-EXISTING").unwrap();
        install_defaults_to(&bundled, false).expect("capture bundled contents");
        let bundled_default = bundled.themes.join("default.toml");
        let expected = std::fs::read_to_string(&bundled_default).expect("read bundled");

        // When installing with overwrite enabled.
        install_defaults_to(&destinations, true).expect("install");

        // Then the overwritten file matches the bundled contents, not the stale value.
        let contents = std::fs::read_to_string(&existing).expect("read");
        assert_eq!(contents, expected);
    }

    #[test]
    fn install_idempotent_under_force() {
        // Given a fully-installed destinations dir (files already match the bundled bytes).
        let (destinations, _temps) = fresh_destinations();
        install_defaults_to(&destinations, false).expect("first install");

        // When installing again with overwrite enabled.
        let second = install_defaults_to(&destinations, true).expect("force install");

        // Then every outcome is Overwritten — overwrite rewrites unconditionally,
        // with no content-diff short-circuit that would report Skipped.
        assert!(
            second
                .iter()
                .all(|o| matches!(o, InstallOutcome::Overwritten(_))),
            "force run must overwrite everything, even unchanged files"
        );
    }
}
