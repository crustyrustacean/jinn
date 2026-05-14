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
pub mod persona_scan_actor;

pub use ensure::{EnsurePersonaError, ensure_personas_dir_with_seed};
pub use loader::{parse_persona_file, scan_personas_dir};
pub use persona::Persona;
pub use persona_entry::PersonaEntry;

use std::path::PathBuf;

use crate::common::app_info::APP_NAME;

/// Returns the default personas directory: `~/.config/nullslop/personas/`.
#[must_use]
pub fn personas_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_NAME)
        .join("personas")
}

/// File name for the seed coding-assistant persona.
const SEED_FILENAME: &str = "coding-assistant.md";

/// Returns the seed persona content — modeled on pi-mono's default system prompt.
fn seed_content() -> String {
    include_str!("coding-assistant.md").to_owned()
}
