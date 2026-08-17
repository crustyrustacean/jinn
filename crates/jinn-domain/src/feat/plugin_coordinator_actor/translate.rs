//! Translation from plugin wire types to core domain types.
//!
//! The wire types ([`jinn_plugin_api::ThemeDef`],
//! [`jinn_plugin_api::PersonaDef`]) are frozen public contract; the core
//! types (`jinn_theme::Theme`, the domain `Persona`) refactor freely. This
//! module is the only place the two meet.
//!
//! Translation reuses the core theme parser: a `ThemeDef`'s slot-key →
//! color-string map is rendered as a theme TOML document and parsed back
//! through `ThemeFile`, so contributed themes accept exactly the color
//! formats on-disk themes accept (ANSI name, ANSI code, hex, RGB). A
//! `ThemeDef` with any unparseable slot drops that one theme — never the
//! whole batch, never the host.

use std::fmt::Write as _;

use jinn_plugin_api::{PersonaDef, ThemeDef};
use jinn_theme::theme::ThemeFile;

use crate::feat::persona::Persona;

/// Translates a batch of contributed theme definitions into resolved core
/// themes.
///
/// Empty/malformed definitions are dropped individually. The reserved
/// `"default"` name is allowed: the picker pins the built-in default
/// first and lets a contributed `"default"` restyle it (matching the
/// user-overrides-system rule for the seeded `default.toml`). The output
/// preserves input order.
#[must_use]
pub fn themes(defs: &[ThemeDef]) -> Vec<(String, Option<String>, Theme)> {
    defs.iter()
        .filter(|d| !d.name.trim().is_empty())
        .filter_map(translate_one)
        .collect()
}

/// Translates one definition, or `None` if any slot fails to parse.
fn translate_one(def: &ThemeDef) -> Option<(String, Option<String>, Theme)> {
    let mut table = String::new();
    for (slot, color) in &def.colors {
        // Escape both key and value conservatively: slot keys are validated
        // below, values are arbitrary strings that must round-trip as TOML
        // basic strings.
        if slot.is_empty() || color.contains('\n') {
            return None;
        }
        let _ = writeln!(table, "{slot} = {}", toml_basic_string(color));
    }

    let file: ThemeFile = toml::from_str(&table).ok()?;
    Some((def.name.clone(), def.description.clone(), file.resolve()))
}

/// Renders a string as a TOML basic string (quoted, escaped).
fn toml_basic_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

use jinn_theme::Theme;

/// Translates a batch of contributed persona definitions into domain
/// personas.
///
/// Definitions with an empty or whitespace-only `name` are dropped
/// individually — never the whole batch, never the host. An absent wire
/// description becomes the empty string (the domain persona's picker
/// contract). The output preserves input order.
#[must_use]
pub fn personas(defs: &[PersonaDef]) -> Vec<Persona> {
    defs.iter()
        .filter(|d| !d.name.trim().is_empty())
        .map(|d| Persona {
            name: d.name.clone(),
            description: d.description.clone().unwrap_or_default(),
            body: d.body.clone(),
        })
        .collect()
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

    fn def(name: &str, description: Option<&str>) -> PersonaDef {
        PersonaDef {
            name: name.to_owned(),
            description: description.map(str::to_owned),
            body: "Body text.".to_owned(),
        }
    }

    #[rstest::rstest]
    fn personas_translates_fields() {
        // Given a contributed persona definition with a description.
        // When translating.
        let personas = personas(&[def("coder", Some("Expert coder"))]);

        // Then the domain persona carries the wire fields.
        assert_eq!(personas.len(), 1);
        assert_eq!(personas[0].name, "coder");
        assert_eq!(personas[0].description, "Expert coder");
        assert_eq!(personas[0].body, "Body text.");
    }

    #[rstest::rstest]
    fn personas_maps_missing_description_to_empty() {
        // Given a contributed persona definition without a description.
        // When translating.
        let personas = personas(&[def("minimal", None)]);

        // Then the description is the empty string.
        assert_eq!(personas[0].description, "");
    }

    #[rstest::rstest]
    fn personas_drops_empty_names_individually() {
        // Given a batch with one empty-name and one whitespace-name def.
        // When translating.
        let personas = personas(&[def("", None), def("   ", None), def("kept", None)]);

        // Then only the named def survives, in order.
        assert_eq!(personas.len(), 1);
        assert_eq!(personas[0].name, "kept");
    }

    #[rstest::rstest]
    fn personas_preserves_input_order() {
        // Given an unsorted batch.
        // When translating.
        let personas = personas(&[def("zeta", None), def("alpha", None)]);

        // Then the output order matches the input.
        assert_eq!(personas[0].name, "zeta");
        assert_eq!(personas[1].name, "alpha");
    }
}
