//! Personas - customizable system prompt profiles for the LLM.
//!
//! Persona discovery lives in the persona slice (`jinn-persona`): at
//! wiring, the slice scans the user personas directory
//! (`~/.config/jinn/personas/`), parses the `+++` TOML frontmatter into
//! its cell, and composition publishes the set as a `PersonasLoaded`
//! event; the session actor consumes that event to populate the persona
//! catalog and resolve the active persona. This module holds the domain
//! types (`Persona`, the picker's `PersonaEntry`); each persona defines
//! the agent's identity, behavioral guidelines, and any other system
//! prompt content.

mod persona_entry;

pub use jinn_persona_msg::Persona;
pub use persona_entry::PersonaEntry;
pub use persona_entry::persona_row;
