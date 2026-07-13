//! Stealth settings — anti-detection configuration for the headless browser.
//!
//! Holds the resolved user-agent string, platform identifier, `Accept-Language`
//! header value, Anubis challenge timeout, and optional binary path that the
//! headless Chrome backend applies per tab to avoid cheap bot-detection tells.
//!
//! The user agent is matched to the host OS at compile time so the string never
//! contradicts the platform the browser actually runs on.

use std::path::PathBuf;
use std::time::Duration;

/// The major Chrome version embedded in the derived user-agent strings.
///
/// Kept as a single source of truth so all three OS variants stay in sync.
/// Update periodically against
/// <https://www.whatismybrowser.com/guides/the-latest-user-agent/chrome>.
pub const CHROME_MAJOR: &str = "150.0.0.0";

/// The `Accept-Language` value a real Chrome install sends.
///
/// Headless Chromium ships without one, and the absence is a weighted tell, so
/// the stealth path always sets it.
pub const ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";

/// A realistic, host-OS-matched Chrome user-agent string.
///
/// Rotation within a session is itself a tell, so one stable string is selected
/// per build. A Linux host advertising Windows + SwiftShader is a contradiction
/// detectors weight heavily; matching the real OS avoids that. The string is a
/// `const` so no allocation is needed.
#[must_use]
pub fn derive_user_agent() -> &'static str {
    USER_AGENT
}

#[cfg(target_os = "windows")]
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36";

#[cfg(target_os = "macos")]
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36";

#[cfg(target_os = "linux")]
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36";

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36";

/// The `platform` value paired with [`derive_user_agent`] for the CDP
/// `SetUserAgentOverride` call.
#[must_use]
pub fn derive_platform() -> &'static str {
    PLATFORM
}

#[cfg(target_os = "windows")]
const PLATFORM: &str = "Windows";

#[cfg(target_os = "macos")]
const PLATFORM: &str = "macOS";

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const PLATFORM: &str = "Linux";

/// How long to wait for an Anubis proof-of-work (or similar interstitial) to
/// clear before giving up. The default difficulty (5 leading zeroes) solves in
/// a few seconds on a modern CPU, so 30s is generous.
const DEFAULT_ANUBIS_TIMEOUT: Duration = Duration::from_secs(30);

/// Resolved stealth configuration for one headless browser session.
///
/// Built once from user preferences (and an optional explicit UA override) and
/// cloned cheaply into the factory and each tab's render path. All fields are
/// immutable after construction.
#[derive(Debug, Clone)]
pub struct StealthSettings {
    /// When `false`, no stealth is applied — the fetcher behaves as before.
    pub enabled: bool,
    /// The user-agent string sent in headers and exposed to `navigator.userAgent`.
    pub user_agent: String,
    /// The `navigator.platform` value paired with the user agent.
    pub platform: String,
    /// The `Accept-Language` header value.
    pub accept_language: String,
    /// How long to wait for an interstitial challenge (e.g. Anubis) to clear.
    pub anubis_timeout: Duration,
    /// Optional explicit path to the Chrome/Chromium binary to launch.
    ///
    /// When `None`, the `headless_chrome` crate's own discovery is used.
    pub binary_path: Option<PathBuf>,
}

impl Default for StealthSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            user_agent: derive_user_agent().to_owned(),
            platform: derive_platform().to_owned(),
            accept_language: ACCEPT_LANGUAGE.to_owned(),
            anubis_timeout: DEFAULT_ANUBIS_TIMEOUT,
            binary_path: None,
        }
    }
}

impl StealthSettings {
    /// Builds stealth settings from an optional explicit UA override.
    ///
    /// When `user_agent_override` is `Some`, it replaces the derived UA; the
    /// platform stays derived (the host OS doesn't change just because the UA
    /// string does).
    #[must_use]
    pub fn with_user_agent_override(user_agent_override: Option<&str>) -> Self {
        let mut settings = Self::default();
        if let Some(ua) = user_agent_override {
            ua.clone_into(&mut settings.user_agent);
        }
        settings
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test assertions")]

    use super::*;

    #[test]
    fn derived_user_agent_targets_current_host_os() {
        // Given the host's target_os.
        // When deriving the user agent.
        let ua = derive_user_agent();

        // Then it is the Chrome variant for the current OS.
        #[cfg(target_os = "windows")]
        assert!(ua.contains("Windows NT 10.0; Win64; x64"));
        #[cfg(target_os = "macos")]
        assert!(ua.contains("Macintosh; Intel Mac OS X 10_15_7"));
        #[cfg(target_os = "linux")]
        assert!(ua.contains("X11; Linux x86_64"));
        // And it carries the current Chrome major version.
        assert!(ua.contains(&format!("Chrome/{CHROME_MAJOR} ")));
    }

    #[test]
    fn derived_user_agent_is_stable_across_calls() {
        // Given two calls to derive_user_agent.
        // When comparing.
        // Then the same static string is returned (no rotation).
        assert_eq!(derive_user_agent(), derive_user_agent());
    }

    #[test]
    fn default_settings_use_derived_ua_and_default_timeout() {
        // Given default stealth settings.
        let settings = StealthSettings::default();

        // Then the UA matches the derived one and stealth is enabled.
        assert_eq!(settings.user_agent, derive_user_agent());
        assert!(settings.enabled);
        // And the Anubis timeout is the documented default.
        assert_eq!(settings.anubis_timeout, Duration::from_secs(30));
    }

    #[test]
    fn override_takes_precedence_over_derived_ua() {
        // Given an explicit UA override.
        let custom = "Mozilla/5.0 (Custom; Robot) Custom/1.0";

        // When building settings with the override.
        let settings = StealthSettings::with_user_agent_override(Some(custom));

        // Then the override is used, not the derived UA.
        assert_eq!(settings.user_agent, custom);
    }

    #[test]
    fn override_none_falls_back_to_derived_ua() {
        // Given no override.
        // When building settings.
        let settings = StealthSettings::with_user_agent_override(None);

        // Then the derived UA is used.
        assert_eq!(settings.user_agent, derive_user_agent());
    }

    #[test]
    fn accept_language_is_present_in_defaults() {
        // Given default settings.
        let settings = StealthSettings::default();

        // Then a realistic Accept-Language is set.
        assert!(!settings.accept_language.is_empty());
        assert!(settings.accept_language.starts_with("en-US"));
    }
}
