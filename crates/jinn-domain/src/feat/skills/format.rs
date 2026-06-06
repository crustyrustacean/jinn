//! Skills formatting - produces the `<available_skills>` XML block for the LLM prompt.
//!
//! Uses the Agent Skills standard format. See: <https://agentskills.io/integrate-skills>

use crate::feat::skills::skill::Skill;

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
pub fn format_skills_for_prompt(skills: &[Skill], loaded: &std::collections::HashSet<String>) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let mut lines = vec![
        String::new(),
        "The following skills provide specialized instructions for specific tasks.".to_owned(),
        "Use the skill tool to load a skill's file when the task matches its description.".to_owned(),
        "Skills marked loaded='true' are already in context as a pinned tool result — do not call the skill tool for them.".to_owned(),
        "When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.".to_owned(),
        String::new(),
        "<available_skills>".to_owned(),
    ];

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
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use std::path::PathBuf;

    fn test_skill(name: &str, description: &str) -> Skill {
        Skill {
            name: name.to_owned(),
            description: description.to_owned(),
            body: String::new(),
            file_path: PathBuf::from(format!("/home/user/.agents/skills/{name}/SKILL.md")),
            base_dir: PathBuf::from(format!("/home/user/.agents/skills/{name}")),
        }
    }

    #[rstest::rstest]
    fn format_returns_empty_for_empty_skills() {
        // Given no skills.
        let skills: Vec<Skill> = vec![];

        // When formatting.
        let result = format_skills_for_prompt(&skills, &std::collections::HashSet::new());

        // Then the result is empty.
        assert!(result.is_empty());
    }

    #[rstest::rstest]
    fn format_produces_available_skills_block() {
        // Given one skill.
        let skills = vec![test_skill("my-skill", "A test skill")];

        // When formatting.
        let result = format_skills_for_prompt(&skills, &std::collections::HashSet::new());

        // Then the output contains the XML block.
        assert!(result.contains("<available_skills>"));
        assert!(result.contains("</available_skills>"));
        assert!(result.contains("<name>my-skill</name>"));
        assert!(result.contains("<description>A test skill</description>"));
        assert!(
            result.contains("<location>/home/user/.agents/skills/my-skill/SKILL.md</location>")
        );
    }

    #[rstest::rstest]
    fn format_includes_usage_instructions() {
        // Given one skill.
        let skills = vec![test_skill("test", "Test skill")];

        // When formatting.
        let result = format_skills_for_prompt(&skills, &std::collections::HashSet::new());

        // Then usage instructions are included.
        assert!(result.contains("Use the skill tool"));
        assert!(result.contains("resolve it against the skill directory"));
    }

    #[rstest::rstest]
    fn format_escapes_xml_characters() {
        // Given a skill with special characters.
        let skills = vec![Skill {
            name: "xml-test".to_owned(),
            description: "Has <special> & \"chars\"".to_owned(),
            body: String::new(),
            file_path: PathBuf::from("/path/to/SKILL.md"),
            base_dir: PathBuf::from("/path/to"),
        }];

        // When formatting.
        let result = format_skills_for_prompt(&skills, &std::collections::HashSet::new());

        // Then special characters are escaped.
        assert!(result.contains("&lt;special&gt;"));
        assert!(result.contains("&amp;"));
        assert!(result.contains("&quot;chars&quot;"));
    }

    #[rstest::rstest]
    fn format_handles_multiple_skills() {
        // Given three skills.
        let skills = vec![
            test_skill("skill-a", "First skill"),
            test_skill("skill-b", "Second skill"),
            test_skill("skill-c", "Third skill"),
        ];

        // When formatting.
        let result = format_skills_for_prompt(&skills, &std::collections::HashSet::new());

        // Then all three skills are included.
        assert!(result.contains("<name>skill-a</name>"));
        assert!(result.contains("<name>skill-b</name>"));
        assert!(result.contains("<name>skill-c</name>"));
    }

    #[rstest::rstest]
    fn format_marks_loaded_skill_with_attribute() {
        // Given one loaded and one unloaded skill.
        let skills = vec![
            test_skill("loaded-skill", "A loaded skill"),
            test_skill("unloaded-skill", "An unloaded skill"),
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
        let skills = vec![
            test_skill("a", "A"),
            test_skill("b", "B"),
            test_skill("c", "C"),
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
    fn format_empty_loaded_set_no_attributes() {
        // Given skills with no loaded set.
        let skills = vec![test_skill("foo", "Foo")];

        // When formatting with empty loaded set.
        let result = format_skills_for_prompt(&skills, &std::collections::HashSet::new());

        // Then no <skill> opening tag carries the loaded attribute.
        assert!(
            !result.contains("<skill loaded="),
            "no <skill> tag should carry the loaded attribute when set is empty: {result}"
        );
    }
}
