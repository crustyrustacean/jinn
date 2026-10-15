//! The bundled-resource catalogue.
//!
//! The whole `res/` tree is embedded at compile time by [`include_dir!`], so
//! what `jinn install` ships is *derived* from the tree's shape rather than
//! hand-written. Dropping a file into `res/` ships it; nothing has to be
//! registered anywhere.
//!
//! A resource's **kind** — and therefore its destination root — comes from its
//! top-level directory under `res/` (`themes/`, `personas/`, `prompts/`,
//! `skills/`). Anything beneath that directory is preserved in the destination
//! path, so a nested structure like `skills/<name>/references/<file>.md`
//! installs as itself.
//!
//! A file whose top-level directory is none of the four is a hard error, not a
//! silent skip: silently skipping is the failure this module exists to
//! eliminate.

use std::path::{Path, PathBuf};

use error_stack::{Report, ResultExt};
use include_dir::{Dir, DirEntry, File, include_dir};
use wherror::Error;

use crate::{Destinations, InstallError};

/// Every file under `res/`, embedded at compile time.
///
/// `include_dir!` expands to one `include_bytes!` per file, so *editing* a
/// resource re-triggers a rebuild on its own. *Adding*, moving, or deleting one
/// does not — `build.rs` declares `rerun-if-changed` for the whole tree to
/// close that gap.
static RESOURCES: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../res");

/// Where a bundled resource should be installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A theme, installed to the themes root.
    Theme,
    /// A persona, installed to the personas root.
    Persona,
    /// A prompt template, installed to the prompts root.
    Prompt,
    /// A workflow (one directory per workflow), installed to the workflows
    /// root — the directory workflow discovery scans.
    Workflow,
    /// A skill, installed to the skills root.
    Skill,
}

impl Kind {
    /// The top-level directory under `res/` that selects this kind.
    pub(crate) const fn dir_name(self) -> &'static str {
        match self {
            Kind::Theme => "themes",
            Kind::Persona => "personas",
            Kind::Prompt => "prompts",
            Kind::Workflow => "workflows",
            Kind::Skill => "skills",
        }
    }

    /// Resolves this kind to its destination root within `destinations`.
    pub(crate) fn root(self, destinations: &Destinations) -> &Path {
        match self {
            Kind::Theme => destinations.themes(),
            Kind::Persona => destinations.personas(),
            Kind::Prompt => destinations.prompts(),
            Kind::Workflow => destinations.workflows(),
            Kind::Skill => destinations.skills(),
        }
    }

    /// This kind's position in the install order: themes, then personas, then
    /// prompts, then workflows, then skills.
    const fn rank(self) -> u8 {
        match self {
            Kind::Theme => 0,
            Kind::Persona => 1,
            Kind::Prompt => 2,
            Kind::Workflow => 3,
            Kind::Skill => 4,
        }
    }
}

/// One bundled resource: its kind, its path relative to that kind's
/// destination root, and its compile-time-embedded payload.
#[derive(Debug)]
pub(crate) struct Bundled {
    /// Selects the destination root; see [`Kind`].
    kind: Kind,
    /// Path relative to the destination root, with any nesting beneath the
    /// kind directory preserved (e.g. `default.toml`,
    /// `jinn-usage/references/keybindings.md`).
    relative: PathBuf,
    /// The embedded payload, written verbatim.
    contents: &'static [u8],
}

impl Bundled {
    /// Where this resource installs: its kind's destination root joined with
    /// its path relative to that root.
    pub(crate) fn destination(&self, destinations: &Destinations) -> PathBuf {
        self.kind.root(destinations).join(&self.relative)
    }

    /// The embedded payload, written verbatim.
    pub(crate) fn contents(&self) -> &'static [u8] {
        self.contents
    }
}

/// A resource's top-level directory under `res/` named no known kind, so it has
/// no destination to install into.
#[derive(Debug, Error)]
#[error(debug)]
struct UnknownKind;

/// Every embedded resource, in install order.
///
/// Ordering is by kind (themes, personas, prompts, skills) and then by
/// ascending path within a kind, so the sequence is deterministic and stable
/// across runs over the same tree.
///
/// # Errors
///
/// Returns [`Report<InstallError>`] if any embedded file sits under a
/// top-level directory that names no kind.
pub(crate) fn bundled_catalogue() -> Result<Vec<Bundled>, Report<InstallError>> {
    let mut catalogue = Vec::new();
    collect_files(&RESOURCES, &mut catalogue)?;

    catalogue.sort_by(|a, b| (a.kind.rank(), &a.relative).cmp(&(b.kind.rank(), &b.relative)));

    Ok(catalogue)
}

/// Walks `dir` recursively, appending one [`Bundled`] per embedded file.
///
/// Embedded paths are already relative to the root passed to `include_dir!` —
/// the root itself contributes an empty prefix — so a file's destination comes
/// straight from [`File::path`] and no prefix is accumulated along the way.
fn collect_files(
    dir: &'static Dir<'static>,
    catalogue: &mut Vec<Bundled>,
) -> Result<(), Report<InstallError>> {
    for entry in dir.entries() {
        match entry {
            DirEntry::Dir(nested) => collect_files(nested, catalogue)?,
            DirEntry::File(file) => catalogue.push(resource(file)?),
        }
    }

    Ok(())
}

/// Builds one [`Bundled`] from an embedded file, splitting its path into the
/// kind-selecting top-level directory and the destination-relative remainder.
///
/// [`File::path`] is already rooted at the directory handed to `include_dir!`
/// and the embedded bytes are `'static`, so the destination comes straight
/// from the file — no path prefix is accumulated along the walk.
fn resource(file: &'static File<'static>) -> Result<Bundled, Report<InstallError>> {
    let embedded = file.path();
    let (kind, relative) = split_kind(embedded)
        .change_context(InstallError)
        .attach("bundled resource has no destination")
        .attach(format!("path: {}", embedded.display()))?;

    Ok(Bundled {
        kind,
        relative: relative.to_path_buf(),
        contents: file.contents(),
    })
}

/// Splits a `res/`-relative path into its kind and the path relative to that
/// kind's destination root.
///
/// Matching is component-wise, so `themes-extra/x` does not match the `themes`
/// kind; neither does a file sitting directly in `res/`, which has no top-level
/// directory to name a kind.
fn split_kind(path: &Path) -> Result<(Kind, &Path), UnknownKind> {
    const KINDS: [Kind; 5] = [
        Kind::Theme,
        Kind::Persona,
        Kind::Prompt,
        Kind::Workflow,
        Kind::Skill,
    ];

    KINDS
        .iter()
        .find_map(|kind| {
            path.strip_prefix(kind.dir_name())
                .ok()
                .map(|rest| (*kind, rest))
        })
        .ok_or(UnknownKind)
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

    #[rstest::rstest]
    #[test]
    fn embedded_tree_is_non_empty_and_kind_covered() {
        // Given the embedded catalogue.
        let catalogue = bundled_catalogue().expect("derive catalogue");

        // Then it carries every kind with at least one file, so a missing
        // `res/` subtree at build time cannot ship a partial bundle.
        // (Membership against the repo's on-disk tree is deliberately not
        // asserted here: `res/` is content, not test fixture data.)
        for kind in [
            Kind::Theme,
            Kind::Persona,
            Kind::Prompt,
            Kind::Workflow,
            Kind::Skill,
        ] {
            let count = catalogue.iter().filter(|b| b.kind == kind).count();
            assert!(count > 0, "kind {kind:?} ships no files");
        }
    }

    #[rstest::rstest]
    #[test]
    fn catalogue_is_ordered_by_kind_then_path() {
        // Given the derived catalogue.
        let catalogue = bundled_catalogue().expect("derive catalogue");

        // Then its (kind, path) pairs are non-decreasing.
        let keys: Vec<_> = catalogue
            .iter()
            .map(|b| (b.kind.rank(), &b.relative))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "catalogue must be sorted by kind then path");
    }

    #[rstest::rstest]
    #[test]
    fn catalogue_order_is_stable_across_runs() {
        // Given the catalogue derived once.
        let first = bundled_catalogue().expect("derive catalogue");

        // When deriving it again.
        let second = bundled_catalogue().expect("derive catalogue");

        // Then both derivations agree, entry for entry.
        let order = |catalogue: &[Bundled]| -> Vec<(Kind, PathBuf)> {
            catalogue
                .iter()
                .map(|b| (b.kind, b.relative.clone()))
                .collect()
        };
        assert_eq!(order(&first), order(&second));
    }

    #[rstest::rstest]
    #[case("themes/default.toml", Kind::Theme, "default.toml")]
    #[case("personas/general.md", Kind::Persona, "general.md")]
    #[case("prompts/plan.md", Kind::Prompt, "plan.md")]
    #[case(
        "skills/simple-task-loop/SKILL.md",
        Kind::Skill,
        "simple-task-loop/SKILL.md"
    )]
    fn top_level_directory_selects_the_kind_and_relative_path(
        #[case] embedded: &str,
        #[case] kind: Kind,
        #[case] relative: &str,
    ) {
        // Given a `res/`-relative path.
        // When splitting it.
        let (resolved, rest) = split_kind(Path::new(embedded)).expect("known kind");

        // Then the top-level directory named the kind and the remainder is the
        // destination-relative path.
        assert_eq!(resolved, kind);
        assert_eq!(rest, Path::new(relative));
    }

    #[rstest::rstest]
    #[case("prompts-extra/thing.md")]
    #[case("README.md")]
    fn unmappable_top_level_directory_is_an_error(#[case] path: &str) {
        // Given a path whose top-level directory names no kind.
        // When splitting it.
        let result = split_kind(Path::new(path));

        // Then it is an error, never a silent skip.
        assert!(result.is_err(), "{path} should not resolve to a kind");
    }
}
