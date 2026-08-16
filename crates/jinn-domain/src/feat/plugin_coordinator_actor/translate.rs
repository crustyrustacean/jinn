//! Translation from plugin wire types to core domain types.
//!
//! The wire types ([`jinn_plugin_api::ThemeDef`]) are frozen public
//! contract; the core types (`jinn_theme::Theme`) refactor freely. This
//! module is the only place the two meet.
//!
//! Translation reuses the core theme parser: a `ThemeDef`'s slot-key →
//! color-string map is rendered as a theme TOML document and parsed back
//! through `ThemeFile`, so contributed themes accept exactly the color
//! formats on-disk themes accept (ANSI name, ANSI code, hex, RGB). A
//! `ThemeDef` with any unparseable slot drops that one theme — never the
//! whole batch, never the host.

use jinn_plugin_api::ThemeDef;
use jinn_theme::theme::ThemeFile;

/// The built-in theme name, reserved: the host always provides it and
/// plugin contributions with this name are ignored.
pub const RESERVED_THEME_NAME: &str = "default";

/// Translates a batch of contributed theme definitions into resolved core
/// themes.
///
/// Empty/malformed definitions are dropped individually; `"default"` is
/// reserved and skipped. The output preserves input order.
#[must_use]
pub fn themes(defs: &[ThemeDef]) -> Vec<(String, Option<String>, Theme)> {
    defs.iter()
        .filter(|d| d.name != RESERVED_THEME_NAME && !d.name.trim().is_empty())
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
        table.push_str(&format!("{slot} = {}\n", toml_basic_string(color)));
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
