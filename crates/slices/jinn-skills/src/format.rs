//! Skills formatting - produces the `<available_skills>` XML block for the LLM prompt.
//!
//! Uses the Agent Skills standard format. See: <https://agentskills.io/integrate-skills>

use jinn_skills_msg::Skill;

/// Formats the available skills list as an XML block for inclusion in the LLM prompt.
///
/// Returns an empty string if the skills list is empty.
///
/// Format follows the Agent Skills standard:
///
/// ```xml
/// <available_skills>
///   <skill>
///     <name>skill-name</name>
///     <description>Skill description</description>
///     <location>/path/to/SKILL.md</location>
///   </skill>
/// </available_skills>
/// ```
pub fn format_skills_for_prompt<S>(
    skills: &[Skill],
    loaded: &std::collections::HashSet<String, S>,
) -> String
where
    S: std::hash::BuildHasher,
{
    if skills.is_empty() {
        return String::new();
    }

    let mut lines = vec![
        String::new(),
        "The following skills provide specialized instructions for specific tasks.".to_owned(),
        "Use the skill tool to load a skill's file when the task matches its description.".to_owned(),
        "Skills marked loaded='true' are already in context as a pinned tool result — do not call the skill tool for them.".to_owned(),
        "Skills bundle reference materials in spec-standard scripts/, references/, and assets/ directories beside SKILL.md. Each skill's <base_dir> holds that directory's absolute path; resolve any relative path in a skill body against it.".to_owned(),
        String::new(),
        "<available_skills>".to_owned(),
    ];

    let mut skills = skills.to_vec();
    skills.sort_unstable();

    for skill in skills {
        let loaded_attr = if loaded.contains(&skill.name) {
            " loaded=\"true\""
        } else {
            ""
        };
        lines.push(format!("  <skill{loaded_attr}>"));
        lines.push(format!("    <name>{}</name>", escape_xml(&skill.name)));
        lines.push(format!(
            "    <description>{}</description>",
            escape_xml(&skill.description)
        ));
        lines.push(format!(
            "    <location>{}</location>",
            escape_xml(&skill.file_path.to_string_lossy())
        ));
        lines.push(format!(
            "    <base_dir>{}</base_dir>",
            escape_xml(&skill.base_dir.to_string_lossy())
        ));
        lines.push("  </skill>".to_owned());
    }

    lines.push("</available_skills>".to_owned());
    lines.join("\n")
}

/// Escapes special XML characters in a string.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;

    /// The skill file name, as a typed relative path.
    fn skill_file_name() -> jinn_testutil::MarkedPathBuf<jinn_testutil::Relative> {
        jinn_testutil::MarkedPathBuf::<jinn_testutil::Relative>::new("SKILL.md")
            .expect("skill file name")
    }

    fn test_skill(root: &std::path::Path, name: &str, description: &str) -> Skill {
        Skill {
            name: name.to_owned(),
            description: description.to_owned(),
            body: String::new(),
            file_path: root
                .join(".agents")
                .join("skills")
                .join(name)
                .join("SKILL.md"),
            base_dir: root.join(".agents").join("skills").join(name),
            source: jinn_skills_msg::SkillSource::Global,
        }
    }

    #[rstest::rstest]
    fn format_returns_empty_for_empty_skills() {
        // Given no skills.
        let skills: Vec<Skill> = vec![];

        // When formatting.
        let result = format_skills_for_prompt(&skills, &std::collections::HashSet::new());

        // Then the result is empty.
        assert_eq!(result, "");
    }

    #[rstest::rstest]
    fn format_produces_available_skills_block() {
        // Given one skill.
        let dir = jinn_testutil::tempdir().expect("temp dir");
        let skills = vec![test_skill(dir.path(), "my-skill", "A test skill")];

        // When formatting.
        let result = format_skills_for_prompt(&skills, &std::collections::HashSet::new());

        // Then the output contains the XML block. The expected paths are
        // built component-wise, exactly as the fixture builds them, so both
        // carry the platform's native separators.
        let expected_location = format!(
            "<location>{}</location>",
            dir.path()
                .join(".agents")
                .join("skills")
                .join("my-skill")
                .join("SKILL.md")
                .display()
        );
        let expected_base_dir = format!(
            "<base_dir>{}</base_dir>",
            dir.path()
                .join(".agents")
                .join("skills")
                .join("my-skill")
                .display()
        );
        assert!(result.contains("<available_skills>"));
        assert!(result.contains("</available_skills>"));
        assert!(result.contains("<name>my-skill</name>"));
        assert!(result.contains("<description>A test skill</description>"));
        assert!(result.contains(&expected_location));
        assert!(
            result.contains(&expected_base_dir),
            "<base_dir> should hold the skill's base directory (parent of SKILL.md, no trailing /SKILL.md), got: {result}"
        );
    }

    #[rstest::rstest]
    fn format_includes_usage_instructions() {
        // Given one skill.
        let dir = jinn_testutil::tempdir().expect("temp dir");
        let skills = vec![test_skill(dir.path(), "test", "Test skill")];

        // When formatting.
        let result = format_skills_for_prompt(&skills, &std::collections::HashSet::new());

        // Then usage instructions are included.
        assert!(result.contains("Use the skill tool"));
        assert!(
            result.contains("spec-standard"),
            "preamble should describe spec-standard reference dirs, got: {result}"
        );
        assert!(
            result.contains("<base_dir>"),
            "preamble should point at <base_dir> for relative-link resolution, got: {result}"
        );
    }

    #[rstest::rstest]
    fn format_escapes_xml_characters() {
        // Given a skill with special characters.
        let root = jinn_testutil::paths::TestRoot::new().expect("test root");
        let base_dir = root.join_str("path/to").expect("base dir");
        let skills = vec![Skill {
            name: "xml-test".to_owned(),
            description: "Has <special> & \"chars\"".to_owned(),
            body: String::new(),
            file_path: base_dir
                .join_relative(&skill_file_name().as_marked_path())
                .as_ref()
                .to_path_buf(),
            base_dir: base_dir.as_ref().to_path_buf(),
            source: jinn_skills_msg::SkillSource::Global,
        }];

        // When formatting.
        let result = format_skills_for_prompt(&skills, &std::collections::HashSet::new());

        // Then special characters are escaped.
        assert!(result.contains("&lt;special&gt;"));
        assert!(result.contains("&amp;"));
        assert!(result.contains("&quot;chars&quot;"));
    }

    #[rstest::rstest]
    fn format_escapes_base_dir_special_chars() {
        // Given a skill whose base_dir path contains XML-special characters.
        let root = jinn_testutil::paths::TestRoot::new().expect("test root");
        let base_dir = root.join_str("a&b<c>").expect("base dir");
        let base_dir_str = base_dir.as_ref().to_string_lossy().into_owned();
        let skills = vec![Skill {
            name: "weird-path".to_owned(),
            description: "d".to_owned(),
            body: String::new(),
            file_path: base_dir
                .join_relative(&skill_file_name().as_marked_path())
                .as_ref()
                .to_path_buf(),
            base_dir: base_dir.as_ref().to_path_buf(),
            source: jinn_skills_msg::SkillSource::Global,
        }];

        // When formatting.
        let result = format_skills_for_prompt(&skills, &std::collections::HashSet::new());

        // Then the <base_dir> value carries the path in its XML-escaped
        // form: every special character is replaced by its entity.
        let escaped = base_dir_str
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;");
        assert!(
            result.contains(&format!("<base_dir>{escaped}</base_dir>")),
            "base_dir special chars should be escaped, got: {result}"
        );
        // And the raw form never appears.
        assert!(
            !result.contains(&base_dir_str),
            "base_dir must not carry the raw special characters, got: {result}"
        );
    }

    #[rstest::rstest]
    fn format_handles_multiple_skills() {
        // Given three skills.
        let dir = jinn_testutil::tempdir().expect("temp dir");
        let root = dir.path();
        let skills = vec![
            test_skill(root, "skill-a", "First skill"),
            test_skill(root, "skill-b", "Second skill"),
            test_skill(root, "skill-c", "Third skill"),
        ];

        // When formatting.
        let result = format_skills_for_prompt(&skills, &std::collections::HashSet::new());

        // Then all three skills are included.
        assert!(result.contains("<name>skill-a</name>"));
        assert!(result.contains("<name>skill-b</name>"));
        assert!(result.contains("<name>skill-c</name>"));
    }

    #[rstest::rstest]
    fn format_returns_stable_skill_list() {
        // Given three skills.
        let dir = jinn_testutil::tempdir().expect("temp dir");
        let root = dir.path();
        let skills = vec![
            test_skill(root, "skill-a", "First skill"),
            test_skill(root, "skill-d", "4"),
            test_skill(root, "skill-e", "5"),
            test_skill(root, "skill-f", "6"),
            test_skill(root, "skill-g", "7"),
            test_skill(root, "skill-b", "Second skill"),
            test_skill(root, "skill-c", "Third skill"),
        ];

        // When formatting.
        let initial_block = format_skills_for_prompt(&skills, &std::collections::HashSet::new());

        for _ in 0..100 {
            let block = format_skills_for_prompt(&skills, &std::collections::HashSet::new());

            // Then it's the same as the initial block
            assert_eq!(block, initial_block);
        }
    }

    #[rstest::rstest]
    fn format_marks_loaded_skill_with_attribute() {
        // Given one loaded and one unloaded skill.
        let dir = jinn_testutil::tempdir().expect("temp dir");
        let root = dir.path();
        let skills = vec![
            test_skill(root, "loaded-skill", "A loaded skill"),
            test_skill(root, "unloaded-skill", "An unloaded skill"),
        ];
        let mut loaded = std::collections::HashSet::new();
        loaded.insert("loaded-skill".to_owned());

        // When formatting.
        let result = format_skills_for_prompt(&skills, &loaded);

        // Then only the loaded skill has the attribute.
        assert!(
            result.contains("  <skill loaded=\"true\">"),
            "loaded skill should carry loaded='true' attribute: {result}"
        );
        assert!(
            result.contains("  <skill>"),
            "unloaded skill should have no attribute: {result}"
        );
    }

    #[rstest::rstest]
    fn format_loaded_instruction_only_appears_once() {
        // Given several skills.
        let dir = jinn_testutil::tempdir().expect("temp dir");
        let root = dir.path();
        let skills = vec![
            test_skill(root, "a", "A"),
            test_skill(root, "b", "B"),
            test_skill(root, "c", "C"),
        ];
        let mut loaded = std::collections::HashSet::new();
        loaded.insert("a".to_owned());
        loaded.insert("c".to_owned());

        // When formatting.
        let result = format_skills_for_prompt(&skills, &loaded);

        // Then the instruction line appears exactly once.
        assert_eq!(
            result.matches("Skills marked loaded='true'").count(),
            1,
            "instruction line should appear exactly once: {result}"
        );
    }

    #[rstest::rstest]
    fn format_preamble_instruction_appears_exactly_once() {
        // Given several skills.
        let dir = jinn_testutil::tempdir().expect("temp dir");
        let root = dir.path();
        let skills = vec![
            test_skill(root, "a", "A"),
            test_skill(root, "b", "B"),
            test_skill(root, "c", "C"),
        ];

        // When formatting.
        let result = format_skills_for_prompt(&skills, &std::collections::HashSet::new());

        // Then the spec-standard reference-materials instruction appears exactly once.
        assert_eq!(
            result.matches("spec-standard").count(),
            1,
            "reference-materials instruction should appear exactly once: {result}"
        );
    }

    #[rstest::rstest]
    fn format_empty_loaded_set_no_attributes() {
        // Given skills with no loaded set.
        let dir = jinn_testutil::tempdir().expect("temp dir");
        let skills = vec![test_skill(dir.path(), "foo", "Foo")];

        // When formatting with empty loaded set.
        let result = format_skills_for_prompt(&skills, &std::collections::HashSet::new());

        // Then no <skill> opening tag carries the loaded attribute.
        assert!(
            !result.contains("<skill loaded="),
            "no <skill> tag should carry the loaded attribute when set is empty: {result}"
        );
    }
}
