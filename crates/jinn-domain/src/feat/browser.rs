//! Shared browser configuration — the single home for launch settings consumed
//! by both the `web-fetch` and `web-search` tools when their backend selects a
//! browser (`headless-chrome` or `headed-chrome`).
//!
//! This module holds:
//! - [`BrowserConfig`] — serialized as `[browser]` in `jinn.toml`. Covers the
//!   binary to launch, an optional explicit user-agent override, and the
//!   interstitial-challenge timeout. It deliberately does NOT select headed vs
//!   headless — that is a per-tool `backend` decision (see [`BrowserBackend`]).
//! - [`BrowserBinary`] — which binary to launch.
//! - [`BrowserBackend`] — the shared `http | headless-chrome | headed-chrome`
//!   selector, used by both `[web_fetch].backend` and `[web_search].backend`.
//!
//! Conversion into [`StealthSettings`] lives here so both tools resolve the
//! same launch flags from the same source.

use std::time::Duration;

use jinn_web_fetch::stealth::StealthSettings;
use serde::{Deserialize, Serialize};

/// Browser backend selection, shared by `web-fetch` and `web-search`.
///
/// Each tool independently picks one of these as its `backend`. `http` runs no
/// browser; `headless-chrome` and `headed-chrome` source their launch
/// configuration from the shared [`BrowserConfig`].
///
/// Serialized as kebab-case strings in `jinn.toml` (`http`, `headless-chrome`,
/// `headed-chrome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BrowserBackend {
    /// Plain HTTP via `reqwest`. No browser, no JavaScript rendering.
    Http,
    /// Headless Chromium: no visible window, throwaway or persistent profile.
    HeadlessChrome,
    /// Headed Chromium: a visible window with a persistent profile, warmable
    /// by manually solving interstitial challenges.
    HeadedChrome,
}

/// Which browser binary the browser backends launch.
///
/// `Auto` prefers an installed branded Google Chrome (which carries the codecs
/// and fingerprint of real Chrome) and falls back to the `headless_chrome`
/// crate's bundled Chromium. `Chrome`/`Chromium` force a specific binary; the
/// launch fails with a clear dashboard error if absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum BrowserBinary {
    /// Prefer installed Google Chrome; fall back to bundled Chromium.
    #[default]
    Auto,
    /// Require an installed Google Chrome; error if missing.
    Chrome,
    /// Use the bundled/auto-discovered Chromium.
    Chromium,
}

/// Shared browser launch configuration.
///
/// Serialized as `[browser]` in `jinn.toml`. Consumed by both `web-fetch` and
/// `web-search` whenever their `backend` is `headless-chrome` or
/// `headed-chrome`. Ignored by the `http` backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrowserConfig {
    /// Which browser binary to launch. Default: `"auto"`.
    #[serde(default)]
    pub binary: BrowserBinary,
    /// Optional explicit user-agent override. When `None`, an OS-matched
    /// Chrome user agent is derived from the detected binary version at
    /// startup. Applies to both the browser path and the `http` path.
    #[serde(default)]
    pub user_agent: Option<String>,
    /// Seconds to wait for an interstitial challenge (Anubis PoW, Cloudflare)
    /// to clear before failing. Default: `30`.
    #[serde(default = "default_anubis_timeout_secs")]
    pub anubis_timeout_secs: u64,
    /// Seconds to wait for a human to solve a detected bot challenge in the
    /// headed browser before failing. Default: `120`.
    #[serde(default = "default_challenge_wait_secs")]
    pub challenge_wait_secs: u64,
    /// Seconds to let an empty-looking page settle (slow SPA fill) before the
    /// behavioral challenge fallback renders a verdict. Default: `5`.
    #[serde(default = "default_settle_secs")]
    pub settle_secs: u64,
    /// Keep browser tabs open after a successful read instead of closing
    /// them. Default: `false` (close after read).
    #[serde(default)]
    pub keep_tabs_open: bool,
}

fn default_anubis_timeout_secs() -> u64 {
    30
}

fn default_challenge_wait_secs() -> u64 {
    120
}

fn default_settle_secs() -> u64 {
    5
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            binary: BrowserBinary::Auto,
            user_agent: None,
            anubis_timeout_secs: default_anubis_timeout_secs(),
            challenge_wait_secs: default_challenge_wait_secs(),
            settle_secs: default_settle_secs(),
            keep_tabs_open: false,
        }
    }
}

impl From<&BrowserConfig> for StealthSettings {
    fn from(config: &BrowserConfig) -> Self {
        let mut settings = StealthSettings::with_user_agent_override(config.user_agent.as_deref());
        // Stealth is always enabled: a tool selects "no browser" by choosing
        // the `http` backend, not by disabling stealth. Any browser-backed
        // path wants the anti-detection flags applied.
        settings.enabled = true;
        settings.anubis_timeout = Duration::from_secs(config.anubis_timeout_secs);
        settings.challenge_wait = Duration::from_secs(config.challenge_wait_secs);
        settings.settle = Duration::from_secs(config.settle_secs);
        settings.keep_tabs_open = config.keep_tabs_open;
        // binary_path is resolved later by the BrowserBinaryScanActor and
        // injected into the settings the browser is constructed with; this
        // config-to-settings conversion does not touch the filesystem.
        settings
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test assertions")]

    use super::*;

    #[rstest::rstest]
    #[test]
    fn default_config_uses_documented_challenge_defaults() {
        // Given no input.
        // When constructing the default config.
        let config = BrowserConfig::default();

        // Then the challenge defaults are set.
        assert_eq!(config.challenge_wait_secs, 120);
        assert_eq!(config.settle_secs, 5);
        assert!(!config.keep_tabs_open);
    }

    #[rstest::rstest]
    #[test]
    fn config_round_trips_challenge_fields_through_toml() {
        // Given a custom config.
        let config = BrowserConfig {
            challenge_wait_secs: 60,
            settle_secs: 2,
            keep_tabs_open: true,
            ..BrowserConfig::default()
        };

        // When serializing then deserializing.
        let toml = toml::to_string(&config).expect("serialize");
        let back: BrowserConfig = toml::from_str(&toml).expect("deserialize");

        // Then the custom values survive.
        assert_eq!(back, config);
    }

    #[rstest::rstest]
    #[test]
    fn config_fills_challenge_defaults_when_empty() {
        // Given an empty TOML table.
        // When deserializing.
        let config: BrowserConfig = toml::from_str("").expect("deserialize");

        // Then defaults are filled in.
        assert_eq!(config, BrowserConfig::default());
    }

    #[rstest::rstest]
    #[test]
    fn from_config_populates_challenge_settings() {
        // Given a config with non-default challenge values.
        let config = BrowserConfig {
            challenge_wait_secs: 90,
            settle_secs: 7,
            keep_tabs_open: true,
            ..BrowserConfig::default()
        };

        // When converting to stealth settings.
        let settings = StealthSettings::from(&config);

        // Then the challenge fields carry over.
        assert_eq!(settings.challenge_wait, Duration::from_secs(90));
        assert_eq!(settings.settle, Duration::from_secs(7));
        assert!(settings.keep_tabs_open);
    }
}
