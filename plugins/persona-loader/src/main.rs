//! The first-party personas plugin.
//!
//! Scans the granted personas directories for `*.md` files, parses each
//! one's `+++` TOML frontmatter (`name`, optional `description`) and body,
//! and pushes the full set to the host as one [`SetPersonaEntries`]
//! contribution. The granted directories arrive in the handshake's
//! [`Welcome`] — the plugin never guesses paths.
//!
//! Wire behavior: `Hello` → (await `Welcome`) → one `SetPersonaEntries` →
//! exit. The host keeps the contribution (published as `PersonasLoaded`)
//! after guest end.
//!
//! Parse rules mirror the frozen on-disk format the host loader used:
//! content must start (after leading whitespace) with `+++`, split at the
//! first `\n+++`, the middle is TOML, the body is the remainder with
//! leading newlines stripped and trailing whitespace trimmed. A file that
//! fails to parse is skipped with a note on stderr — one bad file never
//! drops the batch.

use std::collections::BTreeMap;
use std::path::Path;

use error_stack::ResultExt as _;
use jinn_plugin_api::{PersonaDef, PluginToHost, SetPersonaEntries};
use jinn_plugin_sdk::{PluginOutput, hello, push, welcome};
use serde::Deserialize;
use wherror::Error;

/// Persona-file parse failures (carried between parse helpers).
#[derive(Debug, Error)]
#[error(debug)]
pub enum PersonaParseError {
    /// Filesystem I/O failure.
    Io,
    /// Content does not start with `+++` or has no closing `+++`.
    Frontmatter,
    /// TOML parsing failed.
    Parse,
}

/// Frontmatter schema for persona files.
#[derive(Debug, Deserialize)]
struct Frontmatter {
    /// Unique persona name.
    name: String,
    /// Short description.
    #[serde(default)]
    description: String,
}

fn main() {
    let mut out = PluginOutput::stdout();
    if hello(&mut out, "persona-loader").is_err() {
        return note_exit("handshake write failed");
    }
    let Ok(grants) = welcome() else {
        return note_exit("no Welcome from host");
    };

    let personas = collect_personas(&grants.read_dirs);
    if push(
        &mut out,
        PluginToHost::SetPersonaEntries(SetPersonaEntries { personas }),
    )
    .is_err()
    {
        note_exit("contribution write failed");
    }
}

/// Writes a diagnostic to stderr (host-side diagnostics) and returns.
fn note_exit(message: &str) {
    eprintln!("persona-loader: {message}");
}

/// Scans the granted read dirs (earlier dirs shadow same-name later ones,
/// matching the host's user-overrides-system rule) and collects every
/// parseable persona keyed by name, sorted by name. Unparseable files are
/// skipped with a note on stderr — one bad file never drops the batch.
fn collect_personas(read_dirs: &[String]) -> Vec<PersonaDef> {
    let mut defs = BTreeMap::new();
    for dir in read_dirs {
        merge_dir(&mut defs, Path::new(dir));
    }
    defs.into_values().collect()
}

/// Merges one directory's persona files into the accumulated set.
fn merge_dir(defs: &mut BTreeMap<String, PersonaDef>, dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "md") {
            continue;
        }
        match parse_persona_file(&path) {
            Ok(persona) => {
                defs.insert(persona.name.clone(), persona);
            }
            Err(report) => {
                let reason = report.current_context();
                eprintln!(
                    "persona-loader: skipping persona {}: {reason:?}",
                    path.display()
                );
            }
        }
    }
}

/// Parses one persona file from disk.
///
/// # Errors
///
/// Returns an error if the file cannot be read or the frontmatter is malformed.
pub fn parse_persona_file(
    path: &Path,
) -> Result<PersonaDef, error_stack::Report<PersonaParseError>> {
    let content = std::fs::read_to_string(path)
        .change_context(PersonaParseError::Io)
        .attach(format!("failed to read {}", path.display()))?;
    parse_persona_content(&content)
}

/// Parses persona content (testable without the filesystem).
///
/// # Errors
///
/// Returns an error if the content has no `+++` frontmatter or malformed TOML.
pub fn parse_persona_content(
    content: &str,
) -> Result<PersonaDef, error_stack::Report<PersonaParseError>> {
    let trimmed = content.trim_start();

    let Some(after_open) = trimmed.strip_prefix("+++") else {
        return Err(error_stack::Report::new(PersonaParseError::Frontmatter)
            .attach("content must start with +++ frontmatter delimiter"));
    };

    let Some((frontmatter_str, body_rest)) = after_open.split_once("\n+++") else {
        return Err(error_stack::Report::new(PersonaParseError::Frontmatter)
            .attach("missing closing +++ frontmatter delimiter"));
    };

    let frontmatter: Frontmatter = toml::from_str(frontmatter_str.trim())
        .change_context(PersonaParseError::Parse)
        .attach("failed to parse frontmatter TOML")?;

    let body = body_rest.trim_start_matches('\n').trim_end().to_owned();

    Ok(PersonaDef {
        name: frontmatter.name,
        description: if frontmatter.description.is_empty() {
            None
        } else {
            Some(frontmatter.description)
        },
        body,
    })
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

    #[rstest::rstest]
    fn parse_persona_content_with_valid_frontmatter() {
        // Given a valid persona file content.
        let content = "+++\nname = \"coding-assistant\"\ndescription = \"Expert coder\"\n+++\n\nYou are an expert coding assistant.\n";

        // When parsing.
        let persona = parse_persona_content(content).expect("parse");

        // Then fields are correctly extracted.
        assert_eq!(persona.name, "coding-assistant");
        assert_eq!(persona.description.as_deref(), Some("Expert coder"));
        assert_eq!(persona.body, "You are an expert coding assistant.");
    }

    #[rstest::rstest]
    fn parse_persona_content_without_description() {
        // Given a persona without description.
        let content = "+++\nname = \"minimal\"\n+++\n\nBody text here.";

        // When parsing.
        let persona = parse_persona_content(content).expect("parse");

        // Then description is absent on the wire.
        assert_eq!(persona.name, "minimal");
        assert_eq!(persona.description, None);
        assert_eq!(persona.body, "Body text here.");
    }

    #[rstest::rstest]
    fn parse_persona_content_fails_without_frontmatter() {
        // Given content without +++ delimiter.
        let content = "Just some text without frontmatter.";

        // When parsing.
        let result = parse_persona_content(content);

        // Then it fails with Frontmatter.
        assert!(matches!(
            result.unwrap_err().current_context(),
            PersonaParseError::Frontmatter
        ));
    }

    #[rstest::rstest]
    fn parse_persona_content_fails_without_closing_delimiter() {
        // Given content with opening +++ but no closing +++.
        let content = "+++\nname = \"test\"\nNo closing delimiter here.";

        // When parsing.
        let result = parse_persona_content(content);

        // Then it fails with Frontmatter.
        assert!(matches!(
            result.unwrap_err().current_context(),
            PersonaParseError::Frontmatter
        ));
    }

    #[rstest::rstest]
    fn parse_persona_content_fails_with_invalid_toml() {
        // Given content with invalid TOML in frontmatter.
        let content = "+++\nname = invalid toml\n+++\n\nBody.";

        // When parsing.
        let result = parse_persona_content(content);

        // Then it fails with Parse.
        assert!(matches!(
            result.unwrap_err().current_context(),
            PersonaParseError::Parse
        ));
    }

    #[rstest::rstest]
    fn parse_persona_content_preserves_multiline_body() {
        // Given a persona with multiline body.
        let content = "+++\nname = \"multi\"\n+++\n\nLine one.\nLine two.\nLine three.";

        // When parsing.
        let persona = parse_persona_content(content).expect("parse");

        // Then all body lines are preserved.
        assert!(persona.body.contains("Line one."));
        assert!(persona.body.contains("Line two."));
        assert!(persona.body.contains("Line three."));
    }

    #[rstest::rstest]
    fn shipped_learning_tutor_persona_parses_with_its_grounding() {
        // Given the shipped learning-tutor persona bundled into the repo.
        let content = include_str!("../../../res/personas/learning-tutor.md");

        // When parsing it.
        let persona = parse_persona_content(content).expect("parse");

        // Then it keeps its shipped name and an evidence-grounded description.
        assert_eq!(persona.name, "learning-tutor");
        assert!(
            persona
                .description
                .as_deref()
                .is_some_and(|d| d.contains("intelligent-tutoring"))
        );
    }

    #[rstest::rstest]
    fn collect_personas_returns_sorted_personas() {
        // Given a directory with persona files.
        let dir = tempfile::TempDir::new().expect("temp dir");

        std::fs::write(
            dir.path().join("beta.md"),
            "+++\nname = \"beta\"\ndescription = \"B\"\n+++\n\nBeta body.",
        )
        .expect("write");
        std::fs::write(
            dir.path().join("alpha.md"),
            "+++\nname = \"alpha\"\ndescription = \"A\"\n+++\n\nAlpha body.",
        )
        .expect("write");

        // When collecting.
        let personas = collect_personas(&[dir.path().to_string_lossy().into_owned()]);

        // Then personas are sorted by name.
        assert_eq!(personas.len(), 2);
        assert_eq!(personas[0].name, "alpha");
        assert_eq!(personas[1].name, "beta");
    }

    #[rstest::rstest]
    fn collect_personas_returns_empty_for_missing_dir() {
        // Given a nonexistent directory.
        // When collecting.
        let personas = collect_personas(&["/nonexistent/path".to_owned()]);

        // Then an empty vec is returned.
        assert!(personas.is_empty());
    }

    #[rstest::rstest]
    fn collect_personas_skips_invalid_files() {
        // Given a directory with one valid and one invalid file.
        let dir = tempfile::TempDir::new().expect("temp dir");

        std::fs::write(
            dir.path().join("valid.md"),
            "+++\nname = \"valid\"\ndescription = \"V\"\n+++\n\nValid body.",
        )
        .expect("write");
        std::fs::write(dir.path().join("invalid.md"), "Not a valid persona file.").expect("write");

        // When collecting.
        let personas = collect_personas(&[dir.path().to_string_lossy().into_owned()]);

        // Then only the valid persona is returned.
        assert_eq!(personas.len(), 1);
        assert_eq!(personas[0].name, "valid");
    }

    #[rstest::rstest]
    fn collect_personas_ignores_non_md_files() {
        // Given a directory with a .txt file.
        let dir = tempfile::TempDir::new().expect("temp dir");

        std::fs::write(
            dir.path().join("notes.txt"),
            "+++\nname = \"hidden\"\ndescription = \"H\"\n+++\n\nBody.",
        )
        .expect("write");

        // When collecting.
        let personas = collect_personas(&[dir.path().to_string_lossy().into_owned()]);

        // Then no personas are found.
        assert!(personas.is_empty());
    }
}
