//! Stealth settings — anti-detection configuration for the headless browser.
//!
//! Holds the resolved user-agent string, platform identifier, `Accept-Language`
//! header value, Anubis challenge timeout, and optional binary path that the
//! headless Chrome backend applies per tab to avoid cheap bot-detection tells.
//!
//! The user agent is built at runtime from OS-specific prefixes (chosen at
//! compile time via `target_os`) and the detected Chrome major version, so the
//! UA never contradicts the binary that is actually launched.

use std::path::PathBuf;
use std::time::Duration;

/// The Chrome major version used as a fallback when the installed binary's
/// version cannot be probed via `<binary> --version`.
///
/// Kept current so the fallback UA stays realistic. Update periodically
/// against
/// <https://www.whatismybrowser.com/guides/the-latest-user-agent/chrome>.
pub const CHROME_MAJOR: &str = "150";

/// The `Accept-Language` value a real Chrome install sends.
///
/// Headless Chromium ships without one, and the absence is a weighted tell, so
/// the stealth path always sets it.
pub const ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";

/// The OS-specific UA prefix for the host platform.
///
/// Chosen at compile time via `target_os` so it never contradicts the
/// platform the browser actually runs on. The Chrome major version is
/// templated in at runtime (see [`build_user_agent`]).
///
/// A Linux host advertising Windows + SwiftShader is a contradiction
/// detectors weight heavily; matching the real OS avoids that.
#[cfg(target_os = "windows")]
const UA_PREFIX: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/";

#[cfg(target_os = "macos")]
const UA_PREFIX: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/";

#[cfg(target_os = "linux")]
const UA_PREFIX: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/";

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
const UA_PREFIX: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/";

/// The UA suffix appended after the major version. Since Chrome 101's
/// UA-Client-Hints reduction, minor/build/patch are frozen to `0.0.0` in the
/// `navigator.userAgent` string, so only the major varies.
const UA_SUFFIX: &str = ".0.0.0 Safari/537.36";

/// Builds a realistic, host-OS-matched Chrome user-agent string.
///
/// The OS prefix is fixed at compile time; `chrome_major` (e.g. `"138"`) is
/// templated in at runtime so the UA matches the binary actually launched.
/// Rotation within a session is itself a tell, so one string is built per
/// session.
#[must_use]
pub fn build_user_agent(chrome_major: &str) -> String {
    format!("{UA_PREFIX}{chrome_major}{UA_SUFFIX}")
}

/// The user-agent string built with the fallback [`CHROME_MAJOR`].
///
/// Convenience for sites that have not yet resolved a binary version.
#[must_use]
pub fn fallback_user_agent() -> String {
    build_user_agent(CHROME_MAJOR)
}

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

/// Resolved stealth configuration for one browser session.
///
/// Built once from user preferences (and an optional explicit UA override) and
/// cloned cheaply into the factory and each tab's render path. All fields are
/// immutable after construction.
///
/// The `headed` and `profile_dir` fields select the browser mode: `headed`
/// launches a visible browser window; `profile_dir` makes the profile
/// persistent across runs (cookies/clearances survive restart).
#[derive(Debug, Clone)]
pub struct StealthSettings {
    /// When `false`, no stealth is applied — the fetcher behaves as before.
    pub enabled: bool,
    /// Whether to launch a visible (headed) browser window instead of a
    /// headless one. Headed mode is warmable: the user can manually solve
    /// interstitial challenges, and the persistent profile remembers them.
    pub headed: bool,
    /// Optional persistent profile directory passed as `--user-data-dir`.
    ///
    /// When `None`, the browser uses a throwaway temp profile (the current
    /// default behavior). When `Some`, cookies and clearances persist across
    /// runs. Only one process may own a given profile dir at a time
    /// (Chromium's `SingletonLock`); see `SharedBrowser`.
    pub profile_dir: Option<PathBuf>,
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
            headed: false,
            profile_dir: None,
            user_agent: fallback_user_agent(),
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
    fn build_user_agent_targets_current_host_os() {
        // Given the host's target_os.
        // When building a user agent.
        let ua = build_user_agent("138");

        // Then it is the Chrome variant for the current OS.
        #[cfg(target_os = "windows")]
        assert!(ua.contains("Windows NT 10.0; Win64; x64"));
        #[cfg(target_os = "macos")]
        assert!(ua.contains("Macintosh; Intel Mac OS X 10_15_7"));
        #[cfg(target_os = "linux")]
        assert!(ua.contains("X11; Linux x86_64"));
    }

    #[test]
    fn build_user_agent_templates_the_major_version() {
        // Given a detected major version of 138.
        // When building the user agent.
        let ua = build_user_agent("138");

        // Then the major version appears in the string, with the frozen
        // minor/build/patch suffix from Chrome's UA reduction.
        assert!(ua.contains("Chrome/138.0.0.0"));
    }

    #[test]
    fn fallback_user_agent_uses_chrome_major_const() {
        // Given the fallback path.
        // When building the fallback user agent.
        let ua = fallback_user_agent();

        // Then it carries the hardcoded CHROME_MAJOR version.
        assert!(ua.contains(&format!("Chrome/{CHROME_MAJOR}.0.0.0")));
    }

    #[test]
    fn default_settings_use_fallback_ua_and_default_timeout() {
        // Given default stealth settings.
        let settings = StealthSettings::default();

        // Then the UA matches the fallback one and stealth is enabled.
        assert_eq!(settings.user_agent, fallback_user_agent());
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
    fn override_none_falls_back_to_fallback_ua() {
        // Given no override.
        // When building settings.
        let settings = StealthSettings::with_user_agent_override(None);

        // Then the fallback UA is used.
        assert_eq!(settings.user_agent, fallback_user_agent());
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
