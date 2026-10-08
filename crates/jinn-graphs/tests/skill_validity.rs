//! Skill validity: the shipped minigraf skill must be executable truth.
//!
//! Every ```datalog block in `res/skills/minigraf/**.md` is executed against
//! a fresh in-memory minigraf, sequentially, in file order — a block that
//! errors fails the suite. `SKILL.md` frontmatter must parse with jinn's own
//! hand-rolled parser and carry only `name` + `description` (the parser
//! admits nothing else, so extra keys would silently vanish).

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test assertions"
)]

use std::path::PathBuf;

/// Repo root: this test lives at `crates/jinn-graphs/tests/`.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

fn skill_dir() -> PathBuf {
    repo_root().join("res/skills/minigraf")
}

/// All `.md` files in the skill, sorted (SKILL.md first by name order).
fn skill_markdown_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = walk(&skill_dir());
    files.sort();
    files
}

fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(walk(&path));
        } else if path.extension().is_some_and(|ext| ext == "md") {
            found.push(path);
        }
    }
    found
}

/// Extracts the contents of every ```datalog fenced block, in order.
fn datalog_blocks(markdown: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut lines = markdown.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim_start().starts_with("```datalog") {
            let mut block = Vec::new();
            for inner in lines.by_ref() {
                if inner.trim_start().starts_with("```") {
                    break;
                }
                block.push(inner);
            }
            blocks.push(block.join("\n"));
        }
    }
    blocks
}

/// Splits a block into top-level `( ... )` forms by balanced parens,
/// respecting string literals.
fn top_level_forms(block: &str) -> Vec<String> {
    let mut forms = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    let mut in_string = false;
    for ch in block.chars() {
        match ch {
            '"' => {
                in_string = !in_string;
                if depth > 0 {
                    current.push(ch);
                }
            }
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

#[rstest::rstest]
#[test]
fn skill_ships_at_least_one_markdown_file() {
    // Given the bundled skill directory.
    // When listing its markdown.
    let files = skill_markdown_files();

    // Then the skill is actually present in the repo.
    assert!(
        files
            .iter()
            .any(|p| p.file_name().is_some_and(|n| n == "SKILL.md")),
        "SKILL.md missing under res/skills/minigraf"
    );
}

#[rstest::rstest]
#[test]
fn every_datalog_example_in_the_skill_executes() {
    // Given every markdown file in the bundled skill.
    let files = skill_markdown_files();
    let mut checked = 0usize;

    for file in &files {
        let markdown = std::fs::read_to_string(file).expect("read skill file");
        let blocks = datalog_blocks(&markdown);
        assert!(
            !blocks.is_empty(),
            "{} carries no datalog examples; if it teaches Datalog it must show runnable Datalog",
            file.display()
        );

        // When executing each block top-to-bottom against one fresh
        // in-memory minigraf (blocks within a file build on each other;
        // each file starts clean).
        let db = minigraf::Minigraf::in_memory().expect("in-memory db");
        for block in &blocks {
            for form in top_level_forms(block) {
                checked += 1;
                // Then the command is accepted.
                db.execute(&form)
                    .unwrap_or_else(|e| panic!("{} failed: {e}\nform: {form}", file.display()));
            }
        }
    }

    // And the check is not vacuous.
    assert!(
        checked >= 8,
        "expected a meaningful example set, ran {checked}"
    );
}

#[rstest::rstest]
#[test]
fn skill_frontmatter_carries_name_and_description_only() {
    // Given SKILL.md.
    let content = std::fs::read_to_string(skill_dir().join("SKILL.md")).expect("read SKILL.md");

    // When parsing with jinn's hand-rolled frontmatter parser.
    let fm = jinn_skills::frontmatter::parse_frontmatter(&content)
        .expect("SKILL.md must open with --- frontmatter");

    // Then name and description are present (the parser reads only those
    // two keys, so any other key in the block would be dead weight —
    // verified separately below).
    assert_eq!(fm.name.as_deref(), Some("minigraf"));
    let description = fm.description.expect("description present");
    assert!(description.len() > 20, "description is substantive");
}

#[rstest::rstest]
#[test]
fn skill_frontmatter_block_contains_nothing_the_parser_ignores() {
    // Given the raw frontmatter block of SKILL.md.
    let content = std::fs::read_to_string(skill_dir().join("SKILL.md")).expect("read SKILL.md");
    let after = content.trim_start().strip_prefix("---").expect("opens");
    let block = after.split("\n---").next().expect("closes");

    // When reading its keys.
    let keys: Vec<&str> = block
        .lines()
        .filter_map(|line| line.split(':').next())
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .collect();

    // Then only name and description are declared — jinn's parser admits
    // nothing else, so extra keys would silently vanish.
    assert_eq!(keys, vec!["name", "description"]);
}
