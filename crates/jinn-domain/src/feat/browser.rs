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
}

fn default_anubis_timeout_secs() -> u64 {
    30
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            binary: BrowserBinary::Auto,
            user_agent: None,
            anubis_timeout_secs: default_anubis_timeout_secs(),
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
        // binary_path is resolved later by the BrowserBinaryScanActor and
        // injected into the settings the browser is constructed with; this
        // config-to-settings conversion does not touch the filesystem.
        settings
    }
}
