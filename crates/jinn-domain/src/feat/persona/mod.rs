//! Personas - customizable system prompt profiles for the LLM.
//!
//! Personas are markdown files with TOML frontmatter discovered from
//! both user (`~/.config/jinn/personas/`) and system (`/usr/share/jinn/personas/`)
//! directories. User personas override system personas of the same name.
//! Each persona defines the agent's identity, behavioral guidelines, and any
//! other system prompt content.

mod loader;
#[expect(
    clippy::module_inception,
    reason = "persona/mod.rs is the public API, persona/ is implementation"
)]
mod persona;
mod persona_entry;
pub mod persona_scan_actor;

pub use loader::{parse_persona_file, scan_personas_dir, scan_personas_merged};
pub use persona::Persona;
pub use persona_entry::PersonaEntry;
