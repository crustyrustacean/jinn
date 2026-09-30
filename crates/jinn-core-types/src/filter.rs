//! A mode-plus-patterns filter over named resources.
//!
//! One filter type covers both tools and skills, both the user defaults and a
//! saved attendant's, and both directions of policy. Before this there was a
//! blocklist — a set of names to withhold — which could only ever subtract.
//! An attendant that should have a handful of tools could not be expressed,
//! because withholding everything else means enumerating every MCP tool the
//! user's servers contribute, and that enumeration rots the moment a server
//! updates.
//!
//! The filter's patterns are globs over the *namespaced* name, so one entry
//! covers a whole server: `mcp__github__*` withholds every tool the github
//! server contributes without naming any of them.

use std::collections::BTreeSet;

use globset::Glob;
use serde::{Deserialize, Serialize};

/// Whether a filter withholds by omission or by naming.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterMode {
    /// Withhold every name that matches a pattern. Everything else is
    /// permitted. This is the former blocklist, unchanged in meaning.
    #[default]
    Deny,
    /// Permit only names that match a pattern; withhold everything else.
    ///
    /// Absolute, MCP tools included — which is the whole point of it.
    Allow,
}

/// A filter over tool or skill names.
///
/// The default is the empty deny filter, which permits everything: a session
/// with no filter configured behaves exactly as it did before this type
/// existed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameFilter {
    /// Which direction the patterns apply in.
    #[serde(default)]
    pub mode: FilterMode,
    /// The patterns the mode applies.
    ///
    /// `BTreeSet`, not `HashSet`: the config patcher rewrites this array's
    /// bytes on save, and hash iteration order would reshuffle the user's
    /// list between runs.
    ///
    /// An unconfigured filter contributes nothing. The *field* holding it is
    /// what carries `skip_serializing_if`; serde has no container-level form
    /// of that attribute, and an empty filter written out as
    /// `{"mode":"deny"}` would grow a key into every session that configures
    /// nothing.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub names: BTreeSet<String>,
}

impl NameFilter {
    /// A deny filter over `names` — the former blocklist, as a filter.
    ///
    /// This is what the pickers commit. They are blocklist editors by
    /// construction, so they write a blocklist-shaped filter and nothing
    /// else about their behavior changes.
    #[must_use]
    pub fn deny<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            mode: FilterMode::Deny,
            names: names.into_iter().map(Into::into).collect(),
        }
    }

    /// Whether this session may use the resource called `name`.
    ///
    /// # An empty filter is no filter, in both modes
    ///
    /// An allow filter with no patterns would read strictly as "nothing is
    /// permitted", leaving a session with no tools at all. That is almost
    /// never what a user means by writing an empty field, and it fails as a
    /// session that cannot do anything at all rather than as a config error.
    /// Treating empty as "no filter" instead makes a field's absence and its
    /// emptiness agree, which is also what keeps a session with no filter
    /// configured identical to one predating this type.
    ///
    /// This is a deliberate departure from strict allow semantics. It is
    /// here to stay; "fixing" it into the surprise is the bug.
    #[must_use]
    pub fn permits(&self, name: &str) -> bool {
        if self.names.is_empty() {
            return true;
        }
        match self.mode {
            FilterMode::Deny => !self.matches_any(name),
            FilterMode::Allow => self.matches_any(name),
        }
    }

    /// Makes `name` permitted, whichever mode this filter is in.
    ///
    /// Needed because "un-disable" is not one operation across the two
    /// modes: dropping the name from a deny list permits it, and adding it
    /// to an allow list permits it. An allow-mode filter that merely had the
    /// name dropped would deny it — the opposite of the intent.
    pub fn permit(&mut self, name: &str) {
        match self.mode {
            FilterMode::Deny => {
                self.names.remove(name);
            }
            FilterMode::Allow => {
                self.names.insert(name.to_owned());
            }
        }
    }

    /// Makes `name` withheld, whichever mode this filter is in.
    ///
    /// The mirror of [`Self::permit`], and needed for the same reason. In
    /// allow mode this drops every pattern that matches, not just the literal
    /// name: a pattern that covers `name` is what withholds it, and leaving
    /// the pattern in place would withhold it still.
    pub fn withhold(&mut self, name: &str) {
        match self.mode {
            FilterMode::Deny => {
                self.names.insert(name.to_owned());
            }
            FilterMode::Allow => {
                self.names.retain(|pattern| !pattern_matches(pattern, name));
            }
        }
    }

    /// The names in `names` this filter withholds, per its mode.
    ///
    /// Deny mode withholds its own patterns; allow mode withholds everything
    /// they do not cover. This is the lossy view — callers with a finite list
    /// to check, who do not care how the withholding was expressed. A caller
    /// that writes the result back must keep the filter instead: a round trip
    /// through here drops allow mode's patterns.
    #[must_use]
    pub fn withheld_from<'a, I, S>(&self, names: I) -> BTreeSet<String>
    where
        I: IntoIterator<Item = &'a S>,
        S: AsRef<str> + 'a + ?Sized,
    {
        names
            .into_iter()
            .map(|n| n.as_ref())
            .filter(|name| !self.permits(name))
            .map(str::to_owned)
            .collect()
    }

    /// Whether this filter names anything at all.
    ///
    /// The "not configured" test callers use to decide whether a field
    /// should be written or inherited.
    #[must_use]
    pub fn is_unconfigured(&self) -> bool {
        self.names.is_empty()
    }

    /// Whether any pattern covers `name`.
    ///
    /// A pattern equal to `name` short-circuits before any glob work, which
    /// is the overwhelmingly common case for the deny mode the pickers
    /// write. Beyond that the patterns are compiled per call rather than
    /// cached: a session has a handful of patterns and a few dozen tools,
    /// and a cached compiled set would be interior state this type has no
    /// reason to carry through every clone of a profile.
    fn matches_any(&self, name: &str) -> bool {
        self.names
            .iter()
            .any(|pattern| pattern == name || pattern_matches(pattern, name))
    }
}

/// Whether one pattern covers `name`, treating an uncompilable pattern as a
/// literal.
///
/// A typo in a pattern should match nothing, not everything — but silently
/// dropping it would make `tool_filter = { mode = "deny", names = ["bash["] }`
/// withhold all of `bash`, which is the more damaging direction to guess
/// wrong in. Matching the text literally gives the author a filter that does
/// nothing they can see, which is the honest failure.
fn pattern_matches(pattern: &str, name: &str) -> bool {
    Glob::new(pattern).is_ok_and(|glob| glob.compile_matcher().is_match(name))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]

    use super::{FilterMode, NameFilter};

    /// An allow filter over `patterns`.
    fn allow(patterns: &[&str]) -> NameFilter {
        NameFilter {
            mode: FilterMode::Allow,
            names: patterns.iter().map(|p| (*p).to_owned()).collect(),
        }
    }

    /// A deny filter over `patterns`.
    fn deny(patterns: &[&str]) -> NameFilter {
        NameFilter::deny(patterns.iter().map(|p| (*p).to_owned()))
    }

    #[rstest::rstest]
    fn the_default_filter_is_an_empty_deny_filter() {
        // Given no filter at all.
        let filter = NameFilter::default();

        // When reading its mode.
        // Then it is deny, the former blocklist's meaning.
        assert_eq!(filter.mode, FilterMode::Deny);
    }

    #[rstest::rstest]
    fn deny_mode_withholds_a_named_tool() {
        // Given a deny filter naming `bash`.
        let filter = deny(&["bash"]);

        // When asking about `bash`.
        // Then it is withheld.
        assert!(!filter.permits("bash"));
    }

    #[rstest::rstest]
    fn deny_mode_permits_a_name_it_does_not_list() {
        // Given a deny filter naming `bash`.
        let filter = deny(&["bash"]);

        // When asking about `read`.
        // Then it is permitted.
        assert!(filter.permits("read"));
    }

    #[rstest::rstest]
    fn allow_mode_withholds_a_name_it_does_not_list() {
        // Given an allow filter listing only `read`.
        let filter = allow(&["read"]);

        // When asking about `bash`.
        // Then it is withheld — this is what a blocklist could not express.
        assert!(!filter.permits("bash"));
    }

    #[rstest::rstest]
    fn allow_mode_permits_a_listed_name() {
        // Given an allow filter listing only `read`.
        let filter = allow(&["read"]);

        // When asking about `read`.
        // Then it is permitted.
        assert!(filter.permits("read"));
    }

    #[rstest::rstest]
    #[case::server_prefix_matches_its_tools("mcp__github__*", "mcp__github__create_pr", false)]
    #[case::server_prefix_does_not_match_another_server("mcp__github__*", "mcp__gitlab__x", true)]
    #[case::a_literal_matches_itself("read", "read", false)]
    #[case::a_literal_does_not_match_a_longer_name("read", "readdir", true)]
    #[case::a_mid_pattern_star_is_a_wildcard("g*p", "grep", false)]
    #[case::everything_matches_a_star("*", "anything", false)]
    fn a_pattern_withholds_exactly_the_names_it_covers(
        #[case] pattern: &str,
        #[case] name: &str,
        #[case] permitted: bool,
    ) {
        // Given a deny filter over one pattern.
        let filter = deny(&[pattern]);

        // When asking about a name.
        // Then the name is permitted unless the pattern covers it.
        assert_eq!(
            filter.permits(name),
            permitted,
            "pattern {pattern:?} against {name:?}"
        );
    }

    #[rstest::rstest]
    #[case::deny(FilterMode::Deny)]
    #[case::allow(FilterMode::Allow)]
    fn an_empty_filter_permits_everything_in_either_mode(#[case] mode: FilterMode) {
        // Given a filter that names nothing, in this mode.
        let filter = NameFilter {
            mode,
            names: Default::default(),
        };

        // When asking about a tool.
        // Then it is permitted. See `permits`: an empty allow filter
        // withholding everything would leave a session unable to act at all,
        // which is never what an empty field means.
        assert!(filter.permits("bash"));
    }

    #[rstest::rstest]
    fn an_empty_filter_is_unconfigured_in_either_mode() {
        // Given a filter that names nothing, in allow mode.
        let filter = allow(&[]);

        // When asking whether it is configured.
        // Then it is not — a save should omit it, and a restore should
        // inherit rather than pin an empty allow list.
        assert!(filter.is_unconfigured());
    }

    #[rstest::rstest]
    fn a_deny_filter_can_un_withhold_a_name() {
        // Given a deny filter naming `bash` and `edit`.
        let mut filter = deny(&["bash", "edit"]);

        // When making `bash` permitted.
        filter.permit("bash");

        // Then it is permitted and the other name is untouched.
        assert!(filter.permits("bash"));
        assert!(!filter.permits("edit"));
    }

    #[rstest::rstest]
    fn an_allow_filter_can_un_withhold_a_name_only_by_adding_it() {
        // Given an allow filter listing `read` only.
        let mut filter = allow(&["read"]);

        // When making `bash` permitted.
        filter.permit("bash");

        // Then it is permitted — dropping the name would have withheld it,
        // which is the opposite of the intent.
        assert!(filter.permits("bash"));
        assert!(filter.permits("read"));
    }

    #[rstest::rstest]
    fn an_allow_filter_withholds_a_name_by_dropping_the_pattern_that_covers_it() {
        // Given an allow filter covering a whole MCP server.
        let mut filter = allow(&["mcp__github__*", "read"]);

        // When withholding one of that server's tools.
        filter.withhold("mcp__github__create_pr");

        // Then it is withheld even though a pattern still named its prefix —
        // that pattern is what withheld it.
        assert!(!filter.permits("mcp__github__create_pr"));
    }

    #[rstest::rstest]
    fn an_uncompilable_pattern_matches_nothing_rather_than_everything() {
        // Given a deny filter whose pattern is not valid glob syntax.
        let filter = deny(&["bash["]);

        // When asking about a tool sharing that prefix.
        // Then nothing is withheld — a typo fails inertly instead of taking
        // out a whole family of tools the author never named.
        assert!(filter.permits("bash"));
    }

    #[rstest::rstest]
    fn a_filter_round_trips_through_serde() {
        // Given an allow filter over a glob.
        let filter = allow(&["mcp__github__*"]);
        let json = serde_json::to_string(&filter).expect("serializes");

        // When deserialized.
        let restored: NameFilter = serde_json::from_str(&json).expect("deserializes");

        // Then both the mode and the patterns survive.
        assert_eq!(restored, filter);
    }

    #[rstest::rstest]
    fn an_absent_filter_key_deserializes_to_an_empty_deny_filter() {
        // Given JSON carrying no filter, as an older document would.
        let json = r#"{"model":{"single":"ollama/llama3"}}"#;

        // When deserialized into a profile.
        let profile: crate::SessionProfile = serde_json::from_str(json).expect("deserializes");

        // Then the filter permits everything, as before filters existed.
        assert!(profile.tool_filter.permits("bash"));
        assert!(profile.skill_filter.permits("anything"));
    }
}
