//! Graph templates: hand-editable Datalog seed scripts under a per-template
//! directory, split into instantiate-once seed forms and replay-on-open rule
//! forms.

use std::path::{Path, PathBuf};

use error_stack::Report;

use crate::errors::MinigrafServerError;

/// A parsed template script: the forms that run once at instantiation and the
/// forms that run again on every fresh open.
///
/// The split is explicit. A template wraps its rules in one top-level
/// `(rules [...])` form; every other top-level form is a seed form. A script
/// with no `(rules [...])` wrapper is all seed — it has no replay semantics,
/// and nothing guesses that it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateScript {
    /// Forms executed once when a graph is instantiated from this template.
    pub seed_forms: Vec<String>,
    /// `(rule ...)` source forms executed at instantiation **and** re-executed
    /// whenever a graph with this template's provenance is opened into a fresh
    /// handle.
    pub rule_forms: Vec<String>,
}

impl TemplateScript {
    /// Parses `source` into its seed/rule split.
    ///
    /// `;;`-style (any `;`) comments are stripped before splitting. The
    /// top-level forms must each be a `(rules [...])` wrapper (at most one),
    /// a `(transact ...)`, or a `(retract ...)`; anything else is a parse
    /// error naming the offending shape, because a form the server cannot
    /// classify would otherwise silently never run.
    ///
    /// # Errors
    ///
    /// [`MinigrafServerError::BadTemplate`] when the script cannot be split.
    pub fn parse(source: &str) -> Result<Self, Report<MinigrafServerError>> {
        let mut seed_forms = Vec::new();
        let mut rule_forms = Vec::new();
        for form in top_level_forms(source) {
            if let Some(inner) = rules_wrapper_body(&form) {
                for rule in top_level_forms(&inner) {
                    if !is_command(&rule, &["rule"]) {
                        return Err(bad_template(format!(
                            "(rules [...]) may contain only (rule ...) forms, got: {rule}"
                        )));
                    }
                    rule_forms.push(rule);
                }
            } else if is_command(&form, &["transact", "retract"]) {
                seed_forms.push(form);
            } else {
                return Err(bad_template(format!(
                    "template forms must be (transact ...), (retract ...), or one \
                     (rules [...]) wrapper, got: {form}"
                )));
            }
        }
        Ok(Self {
            seed_forms,
            rule_forms,
        })
    }

    /// Executes seed then rule forms against `db`, so an invalid script fails
    /// validation before any graph file is claimed.
    ///
    /// # Errors
    ///
    /// [`MinigrafServerError::BadTemplate`] when any form fails to execute.
    pub fn validate_against(
        &self,
        db: &minigraf::Minigraf,
    ) -> Result<(), Report<MinigrafServerError>> {
        for form in self.seed_forms.iter().chain(self.rule_forms.iter()) {
            db.execute(form)
                .map_err(|e| bad_template(format!("form failed to execute: {e}\nform: {form}")))?;
        }
        Ok(())
    }
}

/// One template directory under the templates root: `<root>/<name>/template.dl`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    /// The template's name — its directory name. Validated like a graph name.
    pub name: String,
    /// Parsed seed/rule split of the template's `template.dl`.
    pub script: TemplateScript,
}

/// Names of the templates under `root`: directories containing a parseable
/// `template.dl`, sorted.
///
/// A directory without `template.dl` is not a template (sibling assets like
/// `steps/*.md` live beside the script and are ignored); a directory with an
/// unparseable script is also not listed — a broken template must not be
/// offered for instantiation.
///
/// # Errors
///
/// [`MinigrafServerError::StoreDir`] when `root` cannot be read; a missing
/// root is an empty list, not an error.
pub fn list_templates(root: &Path) -> Result<Vec<String>, Report<MinigrafServerError>> {
    let mut names = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        // A missing (or never-created) templates root simply has no templates.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(names),
        Err(e) => {
            return Err(Report::new(MinigrafServerError::StoreDir(format!(
                "{}: {e}",
                root.display()
            ))));
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if crate::naming::validate(name).is_err() {
            continue;
        }
        let Some(source) = read_template_source(&path) else {
            continue;
        };
        if TemplateScript::parse(&source).is_ok() {
            names.push(name.to_owned());
        }
    }
    names.sort();
    Ok(names)
}

/// Loads and parses one template by name.
///
/// # Errors
///
/// [`MinigrafServerError::UnknownTemplate`] when the directory or script is
/// missing, [`MinigrafServerError::BadName`] for an unsafe name, and
/// [`MinigrafServerError::BadTemplate`] when the script does not parse.
pub fn load_template(root: &Path, name: &str) -> Result<Template, Report<MinigrafServerError>> {
    crate::naming::validate(name)?;
    let dir = root.join(name);
    let source = read_template_source(&dir).ok_or_else(|| {
        Report::new(MinigrafServerError::UnknownTemplate(format!(
            "`{name}` — no template.dl under {}",
            dir.display()
        )))
    })?;
    Ok(Template {
        name: name.to_owned(),
        script: TemplateScript::parse(&source)?,
    })
}

/// Reads `<dir>/template.dl`, or `None` when absent/unreadable.
fn read_template_source(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join("template.dl")).ok()
}

/// True when `form`'s head symbol is one of `commands`.
fn is_command(form: &str, commands: &[&str]) -> bool {
    let Some(body) = form.trim_start().strip_prefix('(') else {
        return false;
    };
    let head: String = body
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '(' && *c != '[')
        .collect();
    commands.contains(&head.as_str())
}

/// Extracts the inner text of a top-level `(rules [...])` wrapper, or `None`
/// for any other shape.
fn rules_wrapper_body(form: &str) -> Option<String> {
    let trimmed = form.trim();
    if !is_command(trimmed, &["rules"]) {
        return None;
    }
    // Shape is `(rules [ ... ])`: cut between the first `[` and the matching
    // final `]` (indices from `position`/`rposition` are in bounds).
    let chars: Vec<char> = trimmed.chars().collect();
    let open = chars.iter().position(|c| *c == '[')?;
    let close = chars.iter().rposition(|c| *c == ']')?;
    if close <= open {
        return None;
    }
    let inner: String = chars.get(open + 1..close)?.iter().collect();
    Some(inner)
}

/// Splits `source` into top-level `( ... )` forms by balanced parens,
/// skipping `;`-to-end-of-line comments and respecting string literals.
pub(crate) fn top_level_forms(source: &str) -> Vec<String> {
    let mut forms = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    let mut in_string = false;
    let mut in_comment = false;
    for ch in source.chars() {
        if in_comment {
            if ch == '\n' {
                in_comment = false;
            }
            continue;
        }
        match ch {
            // A `;` only starts a comment outside a string literal.
            '"' => {
                in_string = !in_string;
                if depth > 0 {
                    current.push(ch);
                }
            }
            ';' if !in_string => in_comment = true,
            '(' if !in_string => {
                depth += 1;
                current.push(ch);
            }
            ')' if !in_string => {
                depth = depth.saturating_sub(1);
                current.push(ch);
                if depth == 0 {
                    forms.push(std::mem::take(&mut current));
                }
            }
            _ if depth > 0 => current.push(ch),
            _ => {}
        }
    }
    forms
}

/// Builds a [`MinigrafServerError::BadTemplate`] report.
fn bad_template(message: String) -> Report<MinigrafServerError> {
    Report::new(MinigrafServerError::BadTemplate(message))
}

/// The default template root under the app config dir (`~/.config/jinn/graphs`).
pub fn default_templates_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("graphs")
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::indexing_slicing,
        reason = "test fixtures and assertions on known shapes"
    )]

    use super::*;

    fn parse(source: &str) -> TemplateScript {
        TemplateScript::parse(source).expect("parse")
    }

    #[rstest::rstest]
    #[test]
    fn parse_splits_wrapper_rules_from_seed_forms() {
        // Given a script with two seed transacts and a rules wrapper holding
        // two rule forms.
        let script = parse(concat!(
            ";; a comment the parser must skip\n",
            "(transact [[:step/1 :step/status :blocked]])\n",
            "(retract [[:step/1 :step/status :open]])\n",
            "(rules [\n",
            "  (rule [(can-move ?s) [?s :step/status :blocked]])\n",
            "  (rule [(can-move ?s) [?s :step/status :open]])\n",
            "])\n"
        ));

        // When reading the split.
        // Then the two transact/retract forms are seed and the two rule forms
        // are rules, each carrying its full source text.
        assert_eq!(script.seed_forms.len(), 2);
        assert!(script.seed_forms[0].starts_with("(transact"));
        assert!(script.seed_forms[1].starts_with("(retract"));
        assert_eq!(script.rule_forms.len(), 2);
        assert!(script.rule_forms[0].contains("(can-move ?s)"));
        assert!(script.rule_forms[1].contains("(can-move ?s)"));
    }

    #[rstest::rstest]
    #[test]
    fn parse_without_a_wrapper_treats_every_form_as_seed() {
        // Given a script with no (rules [...]) wrapper.
        let script = parse("(transact [[:a :b 1]])\n(transact [[:a :b 2]])\n");

        // Then all forms are seed and none are rules — no wrapper means no
        // replay semantics, never a guess.
        assert_eq!(script.seed_forms.len(), 2);
        assert!(script.rule_forms.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn parse_survives_nested_parens_and_strings() {
        // Given a seed form with a nested list and a string containing
        // parens, quotes-adjacent text, and a semicolon.
        let script = parse(
            "(transact [[:step/1 :step/label \"call (maybe) ; not a comment\" :deps [:x]]])\n",
        );

        // Then the whole form survives as one seed form.
        assert_eq!(script.seed_forms.len(), 1);
        assert!(script.seed_forms[0].contains("call (maybe) ; not a comment"));
    }

    #[rstest::rstest]
    #[test]
    fn parse_rejects_a_form_it_cannot_classify() {
        // Given a script containing a top-level (query ...) form.
        let result = TemplateScript::parse("(query [:find ?e :where [?e _ _]])");

        // Then parsing fails naming the offending shape — a form the server
        // cannot classify must not silently never run.
        let err = result.expect_err("query form must be refused");
        let rendered = format!("{err:?}");
        assert!(rendered.contains("(query"), "rendered: {rendered}");
    }

    #[rstest::rstest]
    #[test]
    fn parse_rejects_a_non_rule_form_inside_the_wrapper() {
        // Given a wrapper containing a transact form.
        let result = TemplateScript::parse("(rules [ (transact [[:a :b 1]]) ])");

        // Then parsing fails: the wrapper is for rules only.
        assert!(result.is_err());
    }

    #[rstest::rstest]
    #[test]
    fn validate_against_runs_forms_against_an_in_memory_db() {
        // Given a parsed script with one seed and one rule.
        let script = parse(concat!(
            "(transact [[:step/1 :step/status :blocked]])\n",
            "(rules [ (rule [(open? ?s) [?s :step/status :open]]) ])\n"
        ));
        let db = minigraf::Minigraf::in_memory().expect("in-memory db");

        // When validating against it.
        script.validate_against(&db).expect("validate");

        // Then the rule registered: a query using it executes.
        db.execute("(query [:find ?s :where (open? ?s)])")
            .expect("rule is registered after validation");
    }

    #[rstest::rstest]
    #[test]
    fn validate_against_fails_when_a_form_is_semantically_invalid() {
        // Given a script whose seed form violates the one-value-per-transact
        // shape minigraf refuses (a malformed pattern).
        let script = parse("(transact [[:a :b]])\n");
        let db = minigraf::Minigraf::in_memory().expect("in-memory db");

        // When validating.
        let result = script.validate_against(&db);

        // Then validation fails naming the failing form.
        let err = result.expect_err("must refuse");
        let rendered = format!("{err:?}");
        assert!(rendered.contains("form failed"), "rendered: {rendered}");
    }

    #[rstest::rstest]
    #[test]
    fn validate_does_not_need_a_graph_file_to_exist() {
        // Given a script and an in-memory db, with no store and no directory
        // at all.
        let script = parse("(transact [[:a :b 1]])\n");
        let db = minigraf::Minigraf::in_memory().expect("in-memory db");

        // When validating.
        // Then it succeeds with nothing on disk — validation never claims a
        // file (ordering pinned again in store tests).
        script.validate_against(&db).expect("validate");
    }

    #[rstest::rstest]
    #[test]
    fn list_templates_lists_parseable_template_directories_sorted() {
        // Given a root with two template dirs (created out of order), a dir
        // without template.dl, and a stray file.
        let root = tempfile::TempDir::new().expect("tempdir");
        for name in ["beta", "alpha"] {
            let dir = root.path().join(name);
            std::fs::create_dir_all(&dir).expect("mkdir");
            std::fs::write(dir.join("template.dl"), "(transact [[:a :b 1]])").expect("write");
        }
        let asset = root.path().join("steps-only");
        std::fs::create_dir_all(asset.join("steps")).expect("mkdir");
        std::fs::write(asset.join("steps/01-do.md"), "do things").expect("write asset");
        std::fs::write(root.path().join("stray.graph"), "").expect("stray file");

        // When listing.
        let names = list_templates(root.path()).expect("list");

        // Then only the two real templates come back, sorted; sibling assets
        // and files are ignored.
        assert_eq!(names, vec!["alpha".to_owned(), "beta".to_owned()]);
    }

    #[rstest::rstest]
    #[test]
    fn list_templates_hides_a_template_whose_script_does_not_parse() {
        // Given a template dir whose script is unparseable.
        let root = tempfile::TempDir::new().expect("tempdir");
        let dir = root.path().join("broken");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("template.dl"), "(query [:find ?e])").expect("write");

        // When listing.
        let names = list_templates(root.path()).expect("list");

        // Then the broken template is not offered.
        assert!(names.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn list_templates_on_a_missing_root_is_an_empty_list() {
        // Given a root that does not exist.
        let root = tempfile::TempDir::new().expect("tempdir");
        let missing = root.path().join("never-created");

        // When listing.
        // Then it is empty, not an error.
        assert_eq!(
            list_templates(&missing).expect("list"),
            Vec::<String>::new()
        );
    }

    #[rstest::rstest]
    #[test]
    fn load_template_parses_the_named_script() {
        // Given a root with one template.
        let root = tempfile::TempDir::new().expect("tempdir");
        let dir = root.path().join("stoplight");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            dir.join("template.dl"),
            "(transact [[:light/1 :state :red]])\n(rules [ (rule [(go ?l) [?l :state :green]]) ])\n",
        )
        .expect("write");

        // When loading it.
        let template = load_template(root.path(), "stoplight").expect("load");

        // Then the name and split come back.
        assert_eq!(template.name, "stoplight");
        assert_eq!(template.script.seed_forms.len(), 1);
        assert_eq!(template.script.rule_forms.len(), 1);
    }

    #[rstest::rstest]
    #[test]
    fn load_template_refuses_an_unknown_name_with_a_listing_remedy() {
        // Given a root with no templates.
        let root = tempfile::TempDir::new().expect("tempdir");

        // When loading a missing template.
        let result = load_template(root.path(), "ghost");

        // Then the error names the template and points at list_templates.
        let err = result.expect_err("must refuse");
        let rendered = format!("{err:?}");
        assert!(rendered.contains("ghost"), "rendered: {rendered}");
        assert!(rendered.contains("list_templates"), "rendered: {rendered}");
    }

    #[rstest::rstest]
    #[test]
    fn load_template_refuses_unsafe_names_before_touching_the_filesystem() {
        // Given a root.
        let root = tempfile::TempDir::new().expect("tempdir");

        // When loading with a traversal name.
        let result = load_template(root.path(), "../escape");

        // Then the name rule refuses it.
        let err = result.expect_err("must refuse");
        let rendered = format!("{err:?}");
        assert!(
            rendered.contains("invalid graph name"),
            "rendered: {rendered}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn full_template_executes_end_to_end_against_a_real_graph_file() {
        // Given a parsed template whose seed has data and whose rules
        // reference it.
        let script = parse(concat!(
            "(transact [[:step/a :edge/to :step/b] [:step/b :edge/to :step/c]])\n",
            "(rules [\n",
            "  (rule [(reach ?x ?y) [?x :edge/to ?y]])\n",
            "  (rule [(reach ?x ?y) [?x :edge/to ?m] (reach ?m ?y)])\n",
            "])\n"
        ));

        // When instantiating the template into a file-backed graph: run it
        // against a fresh handle first (validation), then execute against a
        // real file the way instantiate does.
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("seeded.graph");
        std::fs::File::create(&path).expect("claim");
        let db = minigraf::Minigraf::open(&path).expect("open");
        script.validate_against(&db).expect("validate");

        // Then a transitive rule query resolves across the seeded edges.
        let rows = db
            .execute("(query [:find ?to :where (reach :step/a ?to)])")
            .expect("reach query");
        match rows {
            minigraf::QueryResult::QueryResults { results, .. } => {
                assert_eq!(results.len(), 2, "a->b and a->c: {results:?}");
            }
            other => panic!("expected rows, got {other:?}"),
        }
    }
}
