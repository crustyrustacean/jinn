//! Template validation for shipped TOML config templates.
//!
//! The default config templates (`default_jinn.toml`, `default_providers.toml`)
//! are documentation as much as configuration. This module verifies three
//! independent guarantees about a template, without ever coupling it to the
//! config struct's `Default` values:
//!
//! 1. **Validity** — the template, with every marked example region expanded,
//!    deserializes into the config struct. Whatever a user can produce by
//!    uncommenting the file must parse.
//! 2. **No dead keys** — every key present in the template survives
//!    deserialization (renamed or removed fields would otherwise silently
//!    become unknown-key comments; here they become test failures).
//! 3. **Completeness** — every key path of the config schema appears in the
//!    template, either active or inside a marked example region. Adding a
//!    config field without documenting it fails the test.
//!
//! # Marked example regions
//!
//! A comment line ending in [`ACTIVATION_MARKER`] opens a region: every
//! following TOML-ish comment line (table headers and `key = value` pairs)
//! has one leading `#` stripped, until the next marker or end of file. Prose
//! comment lines pass through untouched, so explanatory text inside a region
//! stays a comment. Regions all expand simultaneously — same-table examples
//! must use distinct entry names or the expanded document is a
//! duplicate-table error (which the validity check catches).
//!
//! # Key-path normalization
//!
//! Maps keyed by a user-chosen name (`[providers.<name>]`,
//! `[mcp_server.<name>.headers]`) and free-form value containers
//! (`extra_body`, plugin `config`) produce different concrete key paths in
//! the schema fixture and the template example. [`normalize`] masks those
//! segments with `*` on both sides so comparison is structural.
//!
//! Array-of-tables (`[[session_lifecycle]]`, `[[auto_prune.regex.rules]]`,
//! `[[providers.<name>.model_info]]`) contribute *no* name segment: their
//! entry names are field values, not path segments, so they need no masking.
//!
//! # Why serde_json for schema enumeration
//!
//! `toml::Value` cannot represent `None` (serialization errors on it), so
//! struct-side key enumeration goes through `serde_json`, which keeps
//! `Option::None` fields as `null` — under their key. A `null` leaf counts
//! as a key-bearing leaf: the field exists and must be documented.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde::Serialize;
use serde::de::DeserializeOwned;
use wherror::Error;

/// Comment suffix that opens a marked example region.
///
/// Every TOML-ish comment line after a line ending in this marker is
/// uncommented (one leading `#` stripped) until the next marker or EOF.
pub const ACTIVATION_MARKER: &str = "(uncomment below to activate)";

/// Failures raised by [`check_template_activates_and_documents`].
#[derive(Debug, Error)]
#[error(debug)]
pub enum TemplateCheckError {
    /// The expanded template is not valid TOML, or does not deserialize
    /// into the target type.
    Parse,
    /// The template carries keys the struct does not know (removed or
    /// renamed fields still documented in the template).
    DeadKeys(Vec<String>),
    /// The schema has keys the template never documents (active or marked).
    MissingKeys(Vec<String>),
}

/// Strip one leading `#` from TOML-ish comment lines inside marker regions.
///
/// A "TOML-ish" line is one that, after stripping the `#`, looks like a
/// table header (`[...]` / `[[...]]`) or a key-value pair. Anything else —
/// prose comments, blank lines, already-active TOML — passes through
/// unchanged. A region closes only at the next marker line or end of file.
#[must_use]
pub fn expand_marked_examples(template: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut in_region = false;
    for line in template.lines() {
        if line.trim_end().ends_with(ACTIVATION_MARKER) {
            out.push(line);
            in_region = true;
            continue;
        }
        if in_region && line.starts_with('#') && line.get(1..).is_some_and(is_toml_ish) {
            // `line.get(1..)` just succeeded, so byte 1 is a char boundary.
            #[expect(
                clippy::string_slice,
                reason = "get(1..) verified the boundary; slicing is infallible"
            )]
            out.push(&line[1..]);
            continue;
        }
        out.push(line);
    }
    let mut result = out.join("\n");
    if template.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Whether a once-commented line is TOML content rather than prose.
///
/// A table header must *end* with `]` (so a prose sentence mentioning
/// `[package.metadata.jinn]` stays prose). A key-value line requires a
/// bare/quoted key, `=`, and a value that *starts* like a TOML value
/// (number, string, array, inline table, or exactly `true`/`false`) — the
/// value check keeps diagram legends like `A = assistant message`
/// (inside auto-prune explainers) classified as prose. There is
/// deliberately no multi-line-continuation rule: a prose line starting
/// with a quote is indistinguishable from an array element without
/// bracket tracking, so templates must keep arrays single-line.
fn is_toml_ish(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with('[') && (trimmed.ends_with(']') || trimmed.ends_with("]]")) {
        return true;
    }
    let Some((key, value)) = trimmed.split_once('=') else {
        return false;
    };
    let key = key.trim();
    if key.is_empty() {
        return false;
    }
    if !matches!(key.chars().next(), Some('"' | '\''))
        && !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    {
        return false;
    }
    // Strip an inline `# comment` before inspecting the value.
    let value = value.split('#').next().unwrap_or("").trim();
    if value == "true" || value == "false" {
        return true;
    }
    let Some(first) = value.chars().next() else {
        return false;
    };
    let numeric = first.is_ascii_digit()
        || (first == '-'
            && value
                .get(1..2)
                .is_some_and(|rest| rest.chars().next().is_some_and(|c| c.is_ascii_digit())));
    numeric || matches!(first, '"' | '\'' | '[' | '{')
}

/// Collect raw key paths from a parsed TOML document.
///
/// Table keys extend the path; scalar leaves end it. Arrays of tables
/// (`[[entry]]`) recurse into their first element so entry fields appear
/// as paths, mirroring the JSON collector; empty tables and empty arrays
/// are themselves leaf paths.
#[must_use]
pub fn collect_toml_key_paths(doc: &toml::Value) -> BTreeSet<Vec<String>> {
    fn walk(value: &toml::Value, prefix: &[String], out: &mut BTreeSet<Vec<String>>) {
        match value {
            toml::Value::Table(table) => {
                if table.is_empty() {
                    out.insert(prefix.to_vec());
                }
                for (key, child) in table {
                    let mut path = prefix.to_vec();
                    path.push(key.clone());
                    walk(child, &path, out);
                }
            }
            toml::Value::Array(items) => match items.first() {
                Some(first) => walk(first, prefix, out),
                None => {
                    out.insert(prefix.to_vec());
                }
            },
            _ => {
                out.insert(prefix.to_vec());
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(doc, &[], &mut out);
    out
}

/// Collect raw key paths from a typed value's JSON representation.
///
/// Object keys recurse; a `null` leaf (an `Option::None` field) still counts
/// as a key-bearing leaf. Arrays recurse into their first element if present
/// (entry shape is position-independent); an empty array is a leaf.
#[must_use]
pub fn collect_json_key_paths(value: &serde_json::Value) -> BTreeSet<Vec<String>> {
    fn walk(value: &serde_json::Value, prefix: &[String], out: &mut BTreeSet<Vec<String>>) {
        match value {
            serde_json::Value::Object(map) => {
                if map.is_empty() {
                    out.insert(prefix.to_vec());
                }
                for (key, child) in map {
                    let mut path = prefix.to_vec();
                    path.push(key.clone());
                    walk(child, &path, out);
                }
            }
            serde_json::Value::Array(items) => match items.first() {
                Some(first) => walk(first, prefix, out),
                None => {
                    out.insert(prefix.to_vec());
                }
            },
            _ => {
                out.insert(prefix.to_vec());
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(value, &[], &mut out);
    out
}

/// Replace user-chosen name segments with `*`.
///
/// `collection_patterns` are full key paths (possibly containing `*`
/// segments) of the collections whose *children* are user-named — maps
/// keyed by name (`providers`, `mcp_server.*.headers`) and free-form value
/// containers (`providers.*.extra_body`). The segment immediately following
/// a matching path becomes `*`, recursively.
#[must_use]
pub fn normalize_paths(
    paths: &BTreeSet<Vec<String>>,
    collection_patterns: &[Vec<String>],
) -> BTreeSet<Vec<String>> {
    fn matches_pattern(pattern: &[String], path: &[String]) -> bool {
        pattern.len() == path.len()
            && pattern
                .iter()
                .zip(path.iter())
                .all(|(p, s)| p == "*" || p == s)
    }

    let mut normalized = BTreeSet::new();
    for path in paths {
        let mut masked: Vec<String> = Vec::with_capacity(path.len());
        let mut mask_next = false;
        for segment in path {
            if mask_next {
                masked.push("*".to_owned());
                mask_next = false;
            } else {
                masked.push(segment.clone());
                mask_next = collection_patterns
                    .iter()
                    .any(|pattern| matches_pattern(pattern, &masked));
            }
        }
        normalized.insert(masked);
    }
    normalized
}

/// The collection key paths whose children are user-chosen names.
///
/// Patterns may contain `*` for already-masked segments. Deliberately
/// *excludes* array-of-tables collections (`aliases`, `session_lifecycle`,
// `project`, `auto_prune.regex.rules`): their entry names are field values,
/// not path segments, so masking them would desynchronize the two sides.
fn collection_patterns() -> &'static Vec<Vec<String>> {
    static PATTERNS: OnceLock<Vec<Vec<String>>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            "providers",
            "mcp_server",
            "plugin",
            "mcp_server.*.headers",
            "providers.*.extra_body",
            "providers.*.model_info.extra_body",
            "plugin.*.config",
        ]
        .into_iter()
        .map(|p| p.split('.').map(str::to_owned).collect())
        .collect()
    })
}

/// Normalize raw key paths with the shared collection pattern list.
#[must_use]
pub fn normalize(paths: &BTreeSet<Vec<String>>) -> BTreeSet<Vec<String>> {
    normalize_paths(paths, collection_patterns())
}

/// Verify a template activates into a valid config and documents every key.
///
/// `schema` is the JSON representation of a fully-specified config fixture
/// (every field present, `Option::None` included); every key path it
/// carries must be documented by the template after normalization.
///
/// # Errors
///
/// Returns [`TemplateCheckError::Parse`] if the expanded template cannot be
/// deserialized as `T`, [`TemplateCheckError::DeadKeys`] if the template
/// carries keys `T` does not know, or [`TemplateCheckError::MissingKeys`] if
/// the schema has keys the template never documents.
pub fn check_template_activates_and_documents<T>(
    template: &str,
    schema: &serde_json::Value,
) -> Result<(), TemplateCheckError>
where
    T: DeserializeOwned + Serialize,
{
    let expanded = expand_marked_examples(template);

    // 1. Validity: what a user gets by uncommenting everything must parse.
    let parsed: T = toml::from_str(&expanded).map_err(|_e| TemplateCheckError::Parse)?;

    // 2. No dead keys: every raw template key must survive deserialization.
    //    serde drops unknown keys; user-map entries survive as data.
    let raw_template_paths = collect_toml_key_paths(
        &toml::from_str::<toml::Value>(&expanded).map_err(|_e| TemplateCheckError::Parse)?,
    );
    let struct_json = serde_json::to_value(&parsed).map_err(|_e| TemplateCheckError::Parse)?;
    let raw_struct_paths = collect_json_key_paths(&struct_json);
    let dead: Vec<String> = raw_template_paths
        .iter()
        .filter(|path| !raw_struct_paths.contains(*path))
        .map(|path| path.join("."))
        .collect();
    if !dead.is_empty() {
        return Err(TemplateCheckError::DeadKeys(dead));
    }

    // 3. Completeness: every normalized schema key must appear in the template.
    let schema_paths = collect_json_key_paths(schema);
    let template_norm = normalize(&raw_template_paths);
    let missing: Vec<String> = normalize(&schema_paths)
        .iter()
        .filter(|path| !template_norm.contains(*path))
        .map(|path| path.join("."))
        .collect();
    if !missing.is_empty() {
        return Err(TemplateCheckError::MissingKeys(missing));
    }
    Ok(())
}

/// Human-readable rendering of a check failure for test assertions.
#[must_use]
pub fn describe(error: &TemplateCheckError) -> String {
    match error {
        TemplateCheckError::Parse => "expanded template does not parse".to_owned(),
        TemplateCheckError::DeadKeys(keys) => format!("dead keys in template: {keys:?}"),
        TemplateCheckError::MissingKeys(keys) => {
            format!("keys missing from template: {keys:?}")
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        clippy::string_slice,
        reason = "test code"
    )]
    use serde::{Deserialize, Serialize};

    use super::*;

    #[rstest::rstest]
    fn marker_lines_open_regions_without_modification() {
        // Given a template with one marker line.
        let template = "# heading\n# example (uncomment below to activate)\n# [tool]\n# name = 1\n";

        // When expanding.
        let expanded = expand_marked_examples(template);

        // Then the marker itself is untouched.
        assert!(
            expanded.contains("(uncomment below to activate)"),
            "marker lost: {expanded}"
        );
        // And the following TOML-ish comment lines are uncommented
        // (leading whitespace after `#` is preserved — still valid TOML).
        assert!(expanded.contains("[tool]"), "header kept: {expanded}");
        assert!(!expanded.contains("# [tool]"), "header still commented");
        assert!(expanded.contains("name = 1"), "kv kept: {expanded}");
    }

    #[rstest::rstest]
    fn prose_comment_lines_inside_region_pass_through() {
        // Given a region containing prose comments between TOML lines.
        let template =
            "# demo (uncomment below to activate)\n# [tool]\n# human note here\n# name = 1\n";

        // When expanding.
        let expanded = expand_marked_examples(template);

        // Then the prose comment stays commented.
        assert!(
            expanded.contains("# human note here"),
            "prose was uncommented: {expanded}"
        );
        // And the TOML lines are still uncommented.
        assert!(expanded.contains("name = 1"));
    }

    #[rstest::rstest]
    fn comments_before_marker_stay_commented() {
        // Given a commented TOML line above any marker.
        let template = "# [early]\n# flag = true\n";

        // When expanding.
        let expanded = expand_marked_examples(template);

        // Then nothing changes.
        assert_eq!(expanded, template);
    }

    #[rstest::rstest]
    fn active_toml_inside_region_is_left_alone() {
        // Given a region that also contains an already-active line.
        let template = "# demo (uncomment below to activate)\n# [tool]\nflag = true\n";

        // When expanding.
        let expanded = expand_marked_examples(template);

        // Then the active line is untouched (only `#`-prefixed lines strip).
        assert!(
            expanded.contains("\nflag = true\n"),
            "active lost: {expanded}"
        );
    }

    #[rstest::rstest]
    fn region_extends_to_end_of_file() {
        // Given a marker with no subsequent marker.
        let template = "# demo (uncomment below to activate)\n# [a]\n# x = 1\n# [b]\n# y = 2\n";

        // When expanding.
        let expanded = expand_marked_examples(template);

        // Then both tables are uncommented.
        assert!(!expanded.contains("# [a]"), "a still commented: {expanded}");
        assert!(!expanded.contains("# [b]"), "b still commented");
        assert!(expanded.contains("y = 2"));
    }

    #[rstest::rstest]
    fn second_marker_opens_new_region_without_closing_expansion_semantics() {
        // Given two adjacent marker regions.
        let template = "# first (uncomment below to activate)\n# [a]\n# x = 1\n# second (uncomment below to activate)\n# [b]\n# y = 2\n";

        // When expanding.
        let expanded = expand_marked_examples(template);

        // Then both regions expand (the marker line itself is not TOML-ish).
        assert!(!expanded.contains("# [a]"), "a still commented: {expanded}");
        assert!(!expanded.contains("# [b]"), "b still commented");
        // And both markers survive.
        assert_eq!(expanded.matches(ACTIVATION_MARKER).count(), 2);
    }

    #[rstest::rstest]
    fn diagram_legend_lines_stay_prose() {
        // Given a region containing an explainer diagram legend
        // (`u = user message` looks like key-value but isn't TOML).
        let template = "# diagram (uncomment below to activate)\n#  u = user message\n#  A = assistant message\n# enabled = true\n";

        // When expanding.
        let expanded = expand_marked_examples(template);

        // Then the legend lines stay commented (their "values" are prose),
        // while the real key-value line is uncommented.
        assert!(
            expanded.contains("#  u = user message"),
            "legend stripped: {expanded}"
        );
        assert!(expanded.contains("#  A = assistant message"));
        assert!(
            !expanded.contains("# enabled = true"),
            "kv kept commented: {expanded}"
        );
    }

    #[rstest::rstest]
    fn multiline_array_continuation_lines_are_uncommented() {
        // Given a region with a multi-line array value.
        let template = "# grants (uncomment below to activate)\n# grants = [\n#   \"<config_dir>/themes\",\n#   \"<data_dir>/notes:w\",\n# ]\n";

        // When expanding.
        let expanded = expand_marked_examples(template);

        // Then the key line, elements, and closing bracket all uncomment.
        assert!(!expanded.contains("# grants = ["), "key kept: {expanded}");
        assert!(
            expanded.contains("\"<config_dir>/themes\","),
            "element kept: {expanded}"
        );
        assert!(expanded.contains(']'), "close bracket kept: {expanded}");
    }

    #[rstest::rstest]
    fn trailing_whitespace_after_marker_still_opens_region() {
        // Given a marker line with trailing spaces.
        let template = "# demo (uncomment below to activate)   \n# [a]\n# x = 1\n";

        // When expanding.
        let expanded = expand_marked_examples(template);

        // Then the region opens.
        assert!(
            !expanded.contains("# [a]"),
            "region did not open: {expanded}"
        );
    }

    #[rstest::rstest]
    fn null_fields_count_as_documented_keys() {
        // Given a JSON schema value where an Option field is null.
        let schema = serde_json::json!({ "section": { "optional": null } });

        // When collecting paths.
        let paths = collect_json_key_paths(&schema);

        // Then the null field's key path is present.
        let key: Vec<String> = vec!["section".to_owned(), "optional".to_owned()];
        assert!(paths.contains(&key), "null field key lost: {paths:?}");
    }

    #[rstest::rstest]
    fn empty_collections_are_leaves_and_populated_arrays_recurse_first_element() {
        // Given a schema with an empty array and a populated one.
        let schema = serde_json::json!({ "empty": [], "entries": [{ "id": 1 }] });

        // When collecting paths.
        let paths = collect_json_key_paths(&schema);

        // Then the empty array is a leaf.
        let empty: Vec<String> = vec!["empty".to_owned()];
        assert!(paths.contains(&empty), "empty array leaf lost: {paths:?}");
        // And the populated array recursed into its first element.
        let id: Vec<String> = vec!["entries".to_owned(), "id".to_owned()];
        assert!(paths.contains(&id), "entry shape lost: {paths:?}");
    }

    #[rstest::rstest]
    fn normalize_masks_user_chosen_name_segments() {
        // Given a template path with a user-chosen server name and a schema
        // path with a different one.
        let template_paths = BTreeSet::from([vec![
            "mcp_server".to_owned(),
            "example".to_owned(),
            "command".to_owned(),
        ]]);
        let schema_paths = BTreeSet::from([vec![
            "mcp_server".to_owned(),
            "fixture".to_owned(),
            "command".to_owned(),
        ]]);

        // When normalizing both.
        let template_norm = normalize(&template_paths);
        let schema_norm = normalize(&schema_paths);

        // Then both collapse to the same wildcarded path.
        let expected = BTreeSet::from([vec![
            "mcp_server".to_owned(),
            "*".to_owned(),
            "command".to_owned(),
        ]]);
        assert_eq!(template_norm, expected);
        assert_eq!(schema_norm, expected);
    }

    #[rstest::rstest]
    fn normalize_masks_recursively_under_wildcarded_collections() {
        // Given a headers path under a user-named server.
        let paths = BTreeSet::from([vec![
            "mcp_server".to_owned(),
            "remote".to_owned(),
            "headers".to_owned(),
            "Authorization".to_owned(),
        ]]);

        // When normalizing.
        let normalized = normalize(&paths);

        // Then both the server name and the header name are masked.
        let expected = BTreeSet::from([vec![
            "mcp_server".to_owned(),
            "*".to_owned(),
            "headers".to_owned(),
            "*".to_owned(),
        ]]);
        assert_eq!(normalized, expected);
    }

    #[rstest::rstest]
    fn normalize_leaves_array_of_tables_entry_fields_unmasked() {
        // Given a rules path (array-of-tables: entry names are field values).
        let paths = BTreeSet::from([vec![
            "auto_prune".to_owned(),
            "regex".to_owned(),
            "rules".to_owned(),
            "pattern".to_owned(),
        ]]);

        // When normalizing.
        let normalized = normalize(&paths);

        // Then no segment is masked.
        let expected: BTreeSet<Vec<String>> = BTreeSet::from([vec![
            "auto_prune".to_owned(),
            "regex".to_owned(),
            "rules".to_owned(),
            "pattern".to_owned(),
        ]]);
        assert_eq!(normalized, expected);
    }

    #[rstest::rstest]
    fn check_reports_missing_schema_keys_by_name() {
        // Given a config type and a template that omits one documented key.
        #[derive(Debug, Serialize, Deserialize)]
        struct Sample {
            known: u32,
            #[serde(default)]
            undocumented: u32,
        }
        let template = "known = 1\n";
        let schema = serde_json::json!({ "known": 1, "undocumented": 2 });

        // When checking.
        let error = check_template_activates_and_documents::<Sample>(template, &schema)
            .expect_err("must fail");

        // Then the missing key is named.
        assert!(
            describe(&error).contains("undocumented"),
            "wrong error: {error:?}"
        );
    }

    #[rstest::rstest]
    fn check_reports_dead_template_keys_by_name() {
        // Given a template documenting a removed field.
        #[derive(Debug, Serialize, Deserialize)]
        struct Sample {
            known: u32,
        }
        let template = "known = 1\nremoved_field = 2\n";
        let schema = serde_json::json!({ "known": 1 });

        // When checking.
        let error = check_template_activates_and_documents::<Sample>(template, &schema)
            .expect_err("must fail");

        // Then the dead key is named.
        assert!(
            describe(&error).contains("removed_field"),
            "wrong error: {error:?}"
        );
    }

    #[rstest::rstest]
    fn check_accepts_template_documenting_every_key() {
        // Given a template covering every schema key, active or marked.
        #[derive(Debug, Serialize, Deserialize)]
        struct Sample {
            active: u32,
            #[serde(default)]
            optional: Option<u32>,
            inner: Inner,
        }
        #[derive(Debug, Serialize, Deserialize)]
        struct Inner {
            flag: bool,
        }
        let template = "active = 1\n\
            # optional section (uncomment below to activate)\n\
            # optional = 7\n\
            [inner]\nflag = true\n";
        let schema = serde_json::json!({
            "active": 1,
            "optional": null,
            "inner": { "flag": true }
        });

        // When checking.
        let result = check_template_activates_and_documents::<Sample>(template, &schema);

        // Then it passes.
        assert!(result.is_ok(), "unexpected failure: {:?}", result.err());
    }

    #[rstest::rstest]
    fn check_fails_when_marked_region_produces_invalid_toml() {
        // Given a template whose marked region duplicates an active table.
        #[derive(Debug, Serialize, Deserialize)]
        struct Sample {
            inner: Inner,
        }
        #[derive(Debug, Serialize, Deserialize)]
        struct Inner {
            flag: bool,
        }
        let template = "[inner]\nflag = true\n\
            # another (uncomment below to activate)\n\
            # [inner]\n# flag = false\n";
        let schema = serde_json::json!({ "inner": { "flag": true } });

        // When checking.
        let result = check_template_activates_and_documents::<Sample>(template, &schema);

        // Then the expansion is rejected as unparseable.
        assert!(matches!(result, Err(TemplateCheckError::Parse)));
    }
}
