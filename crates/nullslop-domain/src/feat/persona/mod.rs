//! Personas — customizable system prompt profiles for the LLM.
//!
//! Personas are markdown files with TOML frontmatter stored in
//! `~/.config/nullslop/personas/`. Each persona defines the agent's identity,
//! behavioral guidelines, and any other system prompt content. A seed
//! "coding-assistant" persona is written on first run.

mod ensure;
mod loader;
mod persona;
mod persona_entry;

pub use ensure::{EnsurePersonaError, ensure_personas_dir_with_seed};
pub use loader::{parse_persona_file, scan_personas_dir};
pub use persona::Persona;
pub use persona_entry::PersonaEntry;

use std::path::PathBuf;

/// Returns the default personas directory: `~/.config/nullslop/personas/`.
#[must_use]
pub fn personas_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("nullslop")
        .join("personas")
}

/// File name for the seed coding-assistant persona.
const SEED_FILENAME: &str = "coding-assistant.md";

/// Returns the seed persona content — modeled on pi-mono's default system prompt.
fn seed_content() -> String {
    let mut content = String::new();
    content.push_str("+++\n");
    content.push_str("name = \"coding-assistant\"\n");
    content.push_str("description = \"Expert coding assistant — the default persona\"\n");
    content.push_str("+++\n\n");
    content.push_str(
        "You are an expert coding assistant. You help users by reading files, executing commands, editing code, and writing new files.\n\n",
    );
    content.push_str("Guidelines:\n");
    content.push_str("- Use bash for file operations like ls, rg, find\n");
    content.push_str("- Be concise in your responses\n");
    content.push_str("- Show file paths clearly when working with files\n");
    content
}
