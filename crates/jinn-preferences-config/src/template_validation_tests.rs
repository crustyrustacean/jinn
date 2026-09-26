//! Template validation for the shipped `default_jinn.toml`.
//!
//! The template is *documentation*, not authority: it is what a first run
//! copies out, and it is not consulted at read time. So the check runs in
//! one direction only — every key the template documents must resolve to a
//! section jinn actually reads, so the file never advertises a key that
//! silently does nothing.
//!
//! The reverse (every section key appears in the template) is deliberately
//! NOT asserted. A section is allowed to be undocumented: the code default
//! is the truth, and forcing every optional key into the template would
//! bury the few that matter.
//!
//! The old aggregate-struct round-trip checks are gone with the deleted
//! `UserPreferences` struct: a section is exercised by the layer's own
//! tests, and a partial section is *not* required to parse into a full
//! struct.

#![allow(
    clippy::expect_used,
    reason = "test code asserts with expect for clear failure messages"
)]

use std::sync::Arc;

use jinn_config::Configurable;

use crate::DEFAULT_CONFIG;
use crate::schemas::{
    AutoPruneConfig, ChatLogConfig, CompactionConfig, CwdSelectorConfig, MinimapConfig,
    ProjectConfig, RequestRetryConfig, SessionLifecycle, SkillsConfig, StallWatchdogConfig,
    ToolCallWatchdogConfig, ToolsConfig, WebSearchConfig,
};

/// Every (fully-qualified) key the template writes, uncommented.
///
/// The template's examples are commented out on purpose, so a plain TOML
/// parse would not see them: both the shipped form and the
/// fully-expanded form are walked as text, tracking the current table so
/// a bare `key = value` is qualified with the section it sits in.
fn template_keys(expanded: bool) -> Vec<String> {
    let text = if expanded {
        jinn_common::template_check::expand_marked_examples(DEFAULT_CONFIG)
    } else {
        DEFAULT_CONFIG.to_owned()
    };
    let mut qualified: Vec<String> = Vec::new();
    // The dotted path of the table the cursor is in. Array-of-tables
    // entries contribute their own header, which is a list section's
    // key, so the prefix is reset rather than extended.
    let mut prefix: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some(inner) = strip_header(line, "[[", "]]") {
            prefix = inner.split('.').map(str::to_owned).collect();
            qualified.push(inner);
            continue;
        }
        if let Some(inner) = strip_header(line, "[", "]") {
            prefix = inner.split('.').map(str::to_owned).collect();
            continue;
        }
        if let Some((key, _)) = line.split_once('=') {
            let key = key.trim().trim_matches('"');
            if key.is_empty() {
                continue;
            }
            let path = if prefix.is_empty() {
                key.to_owned()
            } else {
                format!("{}.{key}", prefix.join("."))
            };
            qualified.push(path);
        }
    }
    qualified
}

/// The text between `open` and `close`, if `line` is exactly that header.
fn strip_header(line: &str, open: &str, close: &str) -> Option<String> {
    let inner = line.strip_prefix(open)?.strip_suffix(close)?;
    Some(inner.trim().to_owned())
}

/// Every key the template ships resolves to a section jinn reads.
#[rstest::rstest]
#[test]
fn template_keys_resolve_to_a_read_section() {
    // Given every key the shipped template writes.
    let keys = template_keys(false);

    // Then each one is a prefix of a registered section key — a table the
    // layer descends into, or a key inside one.
    for key in &keys {
        let owned = key.replace('"', "");
        assert!(
            is_resolvable(&owned),
            "template key `{owned}` is not under any section jinn reads"
        );
    }
}

/// Same, with every commented example expanded.
#[rstest::rstest]
#[test]
fn expanded_template_keys_resolve_to_a_read_section() {
    // Given every key the template writes once examples are uncommented.
    let keys = template_keys(true);

    // Then each one still resolves.
    for key in &keys {
        let owned = key.replace('"', "");
        assert!(
            is_resolvable(&owned),
            "template key `{owned}` is not under any section jinn reads"
        );
    }
}

/// Every section key the layer reads, tables and lists alike.
fn section_keys() -> Vec<&'static str> {
    let mut keys: Vec<&'static str> = vec![
        AutoPruneConfig::KEY,
        CompactionConfig::KEY,
        RequestRetryConfig::KEY,
        ToolsConfig::KEY,
        SkillsConfig::KEY,
        ChatLogConfig::KEY,
        WebSearchConfig::KEY,
        MinimapConfig::KEY,
        CwdSelectorConfig::KEY,
        StallWatchdogConfig::KEY,
        ToolCallWatchdogConfig::KEY,
        jinn_term_msg::prefs::InteractiveTermPrefs::KEY,
        <jinn_mcp_msg::config::McpServersConfig as jinn_config::Configurable>::KEY,
        // The discord section is a leaf `[discord]`; listed by its literal
        // key because jinn-discord depends on this crate, not the reverse.
        "discord",
    ];
    keys.extend([
        <ProjectConfig as jinn_config::ConfigList>::KEY,
        <jinn_tools_msg::CommandPolicyRule as jinn_config::ConfigList>::KEY,
        <SessionLifecycle as jinn_config::ConfigList>::KEY,
    ]);
    keys
}

/// Whether a template key lies at or under a section the layer reads.
fn is_resolvable(key: &str) -> bool {
    section_keys()
        .into_iter()
        .any(|section| key == section || key.starts_with(&format!("{section}.")))
}

/// The list sections in the template use the umbrella spelling, not the
/// pre-umbrella one. A regression here is silent: the old key would parse
/// fine and just do nothing.
#[rstest::rstest]
#[test]
fn template_uses_no_pre_umbrella_spelling() {
    // Given the shipped template's table headers.
    let headers: Vec<String> = template_keys(false)
        .into_iter()
        .filter(|k| k.contains('[') || k.split('.').count() > 1)
        .collect();

    // Then none of the old flat headers is still present.
    for legacy in [
        "[compaction]",
        "[request_retry]",
        "[auto_prune]",
        "[projects]",
        "[session_lifecycle]",
        "[global_command_policy]",
        "[openrouter_web_search]",
        "[cwd_selector]",
        "[minimap]",
        "[stall_watchdog]",
        "[tool_call_watchdog]",
        "[interactive_term]",
    ] {
        let base = legacy.trim_matches(|c| c == '[' || c == ']');
        assert!(
            !headers.iter().any(|h| h == base),
            "template still uses the pre-umbrella header `{legacy}`"
        );
    }
}

/// The shipped template loads into a real layer and every section reads
/// cleanly.
///
/// This is the guarantee the old "template parses as the aggregate"
/// test used to give. It is strictly stronger now: it exercises the real
/// read path (walk the key, merge over the section's Default) for every
/// registered section, rather than one aggregate struct's top-level keys.
#[rstest::rstest]
#[test]
fn the_shipped_template_loads_into_the_configuration_layer() {
    // Given a layer over the shipped template.
    let doc = DEFAULT_CONFIG.parse().expect("template parses as TOML");
    let layer =
        jinn_config::ConfigLayer::load(Arc::new(jinn_config::InMemoryConfigStorage::new(doc)))
            .expect("template loads");

    // When every registered section is read.
    let reads = read_every_section(&layer);

    // Then each one read a value, naming the section that failed.
    for (key, result) in reads {
        assert!(
            result.is_ok(),
            "template section [{key}] did not read: {result:?}"
        );
    }
}

/// Each registered section read off `layer`, as `(key, result)`.
fn read_every_section(
    layer: &jinn_config::ConfigLayer,
) -> Vec<(&'static str, Result<(), jinn_config::ConfigSectionError>)> {
    macro_rules! read {
        ($($t:ty),* $(,)?) => {
            vec![
                $((
                    <$t as jinn_config::Configurable>::KEY,
                    layer.get::<$t>().map(|_| ()),
                )),*
            ]
        };
    }
    read!(
        AutoPruneConfig,
        CompactionConfig,
        RequestRetryConfig,
        ToolsConfig,
        SkillsConfig,
        ChatLogConfig,
        WebSearchConfig,
        MinimapConfig,
        CwdSelectorConfig,
        StallWatchdogConfig,
        ToolCallWatchdogConfig,
        jinn_term_msg::prefs::InteractiveTermPrefs,
        jinn_mcp_msg::config::McpServersConfig,
        DiscordSectionStandIn,
    )
}

/// A stand-in for the discord section, which lives in a crate that
/// depends on *this* one so its impl cannot be named from here.
///
/// The template leaves `[discord]` commented out, so this only needs to
/// prove the key is readable as an absent section — which is exactly the
/// stock-install case the boot panic used to break.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct DiscordSectionStandIn {
    #[serde(default)]
    enabled: bool,
}

impl jinn_config::Configurable for DiscordSectionStandIn {
    const KEY: &'static str = "discord";
}

/// The user-facing migration guide must point only at sections jinn
/// actually reads, or a migrating user lands on a key that silently does
/// nothing.
///
/// Checked against the registered sections rather than the template: the
/// guide is a list of keys, and a key only does something if it lands
/// under a section. Only the old-to-new table is checked — the guide's
/// hand-edit table has prose columns ("Put it under"), not keys.
#[rstest::rstest]
fn the_migration_guide_points_only_at_live_sections() {
    // Given the guide's old-to-new key table.
    let guide = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../doc/jinn-toml-umbrella-migration.md",
    ))
    .expect("the migration guide ships in the repo");
    let destinations = guide_new_spelling(&guide);
    assert!(
        destinations.len() > 10,
        "the guide's key table is missing or unparseable: {destinations:?}"
    );

    // When each destination is resolved to the section that serves it.
    let stale: Vec<&str> = destinations
        .iter()
        .filter(|key| {
            // A key is served when it, or a prefix of it, is a registered
            // section: `tools.disabled` by `[tools]`, and
            // `context_curation.auto_prune.regex.rules` by the
            // `[context_curation.auto_prune]` section that owns the list.
            // An opt-in section with nothing to preconfigure is exempt.
            section_prefixes(key).is_none()
        })
        .map(String::as_str)
        .collect();

    // Then every destination is a key jinn reads.
    assert!(
        stale.is_empty(),
        "migration guide points at keys jinn does not read: {stale:?}"
    );
}

/// The registered section that serves `key`, if any.
///
/// Walks up the dotted path, so a nested key resolves to the registered
/// section on its path that owns it. `None` means no prefix is
/// registered, which is how an opt-in section like `[mcp]` — nothing to
/// preconfigure, so absent from the template — stays exempt.
fn section_prefixes(key: &str) -> Option<&str> {
    let sections = section_keys();
    // Each dot-delimited prefix, longest first: the full key, then
    // `context_curation.auto_prune.regex`, then `context_curation.auto_prune`.
    let mut prefixes = std::iter::successors(Some(key), |shorter| {
        shorter.rsplit_once('.').map(|(head, _)| head)
    });
    prefixes.find(|candidate| sections.contains(candidate))
}

/// The new spelling of every row in the guide's old-to-new key table.
///
/// A row reads `| `[old]`, `[[old.sub]]` | `[new]`, `[[new.sub]]` |`, so a
/// cell may name more than one key. Bracket and angle characters are
/// stripped: `[mcp.<name>]` and `[[session_lifecycle.lifecycle]]` reduce
/// to the bare dotted paths a section is matched on.
fn guide_new_spelling(guide: &str) -> Vec<String> {
    guide
        .lines()
        .filter_map(|row| {
            let rest = row.trim().strip_prefix("| `[")?;
            // The old column ends at the first cell-closing backtick.
            let (_, new_cell) = rest.split_once("` | ")?;
            Some(new_cell.split("` |").next().unwrap_or(new_cell))
        })
        .flat_map(|cell| {
            cell.split(',')
                .map(|key| key.trim_matches(['`', ' ', '[', ']', '<', '>']))
                .filter(|key| !key.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|key| key != "New")
        .collect()
}
