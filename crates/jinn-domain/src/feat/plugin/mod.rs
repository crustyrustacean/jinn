//! User-declared plugins — WASM components jinn runs as child processes.
//!
//! Plugins are declared in `jinn.toml` under `[[plugin]]`. Each entry names
//! a `.wasm` file (path relative to jinn's plugin dir or absolute) plus the
//! capability grants the plugin receives. The coordinator actor spawns one
//! runner child per entry at app start; see `feat/plugin_coordinator_actor`.

use serde::{Deserialize, Serialize};

/// One configured plugin.
///
/// Defined in `jinn.toml` under `[[plugin]]`. The `name` field is the array
/// key the `DocumentPatcher` matches entries by; it also selects the plugin's
/// default scratch dir (`<data_dir>/plugins/<name>/`) and namespaces its
/// contributed state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginConfig {
    /// Unique plugin name. Doubles as the contribution namespace and the
    /// default scratch-dir selector.
    pub name: String,
    /// Path to the plugin's `.wasm` component. Relative paths resolve
    /// against jinn's plugin directory (`<data_dir>/plugins/`).
    pub wasm: String,
    /// Directory paths the plugin may access, as templates (e.g.
    /// `<config_dir>/themes`). See [`PluginPathGrant`]. Every plugin
    /// additionally receives its own writable scratch dir without listing
    /// it here.
    #[serde(default)]
    pub grants: Vec<crate::feat::plugin::PluginPathGrant>,
    /// Whether the plugin may make network requests via `wasi:http`.
    #[serde(default)]
    pub http: bool,
    /// Free-form plugin config passed through to the guest.
    #[serde(default)]
    pub config: Option<toml::Value>,
    /// Set to `false` to disable this plugin without deleting its entry.
    /// The coordinator skips disabled entries entirely.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// The default for [`PluginConfig::enabled`] — `true` unless the user opts
/// out. A separate function (not `#[serde(default)]` on the field) because
/// `bool::default()` is `false`.
#[must_use]
pub fn default_true() -> bool {
    true
}

use std::collections::BTreeMap;
use jinn_theme::Theme;

use crate::feat::plugin_coordinator_actor::protocol::PluginPhase;

/// The plugin contribution cache — data pushed by plugins, held for
/// synchronous consumers (pickers, renderer).
///
/// Written only by the plugin coordinator actor (via [`PluginsCap`]
/// ([`crate::common::tcaps::plugins`]); read by anyone. Contributions are
/// push-only: a stale cache means a dead plugin, never a blocked
/// consumer.
#[derive(Debug, Default)]
pub struct PluginContributions {
    /// Contributed themes, keyed by theme name. The reserved
    /// `"default"` name never appears here (translation drops it).
    themes: BTreeMap<String, ContributedTheme>,
    /// Latest known phase per plugin name.
    phases: BTreeMap<String, PluginPhase>,
}

/// One contributed theme: the resolved core [`Theme`] plus its
/// description.
#[derive(Debug, Clone)]
pub struct ContributedTheme {
    /// The resolved theme.
    pub theme: Theme,
    /// The contributing plugin's description, if any.
    pub description: Option<String>,
    /// The contributing plugin's name (for source-accurate replacement).
    pub source: String,
}

impl PluginContributions {
    /// Replaces the theme set contributed by one plugin.
    pub fn set_themes(&mut self, source: &str, themes: Vec<(String, Option<String>, Theme)>) {
        // Remove this source's previous contributions first: the wire
        // message is a full replacement, not a delta.
        self.themes.retain(|_, t| t.source != source);
        for (name, description, theme) in themes {
            self.themes.insert(
                name,
                ContributedTheme {
                    source: source.to_owned(),
                    theme,
                    description,
                },
            );
        }
    }

    /// All contributed themes, ordered by name.
    pub fn themes(&self) -> impl Iterator<Item = (&str, &ContributedTheme)> {
        self.themes.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// One contributed theme by name.
    #[must_use]
    pub fn theme(&self, name: &str) -> Option<&ContributedTheme> {
        self.themes.get(name)
    }

    /// Records a plugin's latest phase.
    pub fn set_phase(&mut self, name: String, phase: PluginPhase) {
        self.phases.insert(name, phase);
    }

    /// The latest known phase for a plugin.
    #[must_use]
    pub fn phase(&self, name: &str) -> Option<PluginPhase> {
        self.phases.get(name).copied()
    }
}

/// Manifest path grant: a template string plus write intent.
///
/// Defined here (not re-exported from `jinn-plugin`) so `jinn.toml` parsing
/// has no dependency on the runner crate; the coordinator translates to
/// `jinn_plugin::PathGrant` at spawn time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginPathGrant {
    /// Path template (e.g. `<config_dir>/themes`).
    pub path: String,
    /// Grant write access in addition to read.
    #[serde(default)]
    pub writable: bool,
}
