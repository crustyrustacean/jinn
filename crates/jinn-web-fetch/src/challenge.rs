//! Challenge detection — recognize bot-protection interstitials and wait for
//! them to clear.
//!
//! Two known interstitials:
//!
//! - **Anubis** — a proof-of-work challenge. Any real browser running JavaScript
//!   solves it in a few seconds and is redirected through; the only thing the
//!   fetcher must do is _wait_ for the PoW to finish instead of returning early.
//! - **Cloudflare** — "Just a moment..." managed challenge / Turnstile. The
//!   stealth layer may clear it automatically; if not, the wait times out and
//!   surfaces a clear error instead of a silent hang.

use std::time::{Duration, Instant};

use crate::FetchError;

/// The kind of bot-protection interstitial detected on a page, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChallengeKind {
    /// Anubis proof-of-work challenge page.
    Anubis,
    /// Cloudflare "Just a moment..." / Turnstile challenge page.
    Cloudflare,
    /// No challenge detected — the page is real content.
    None,
}

/// Poll interval for [`wait_for_clearance`]. Balances responsiveness against
/// CPU/IO churn while a challenge solves.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Detects a known challenge interstitial in rendered HTML.
///
/// Uses specific DOM markers rather than loose substring matches (e.g. "bot")
/// to avoid false positives on legitimate pages that merely discuss bot
/// protection.
///
/// # Examples
///
/// ```
/// # use jinn_web_fetch::challenge::{detect_challenge, ChallengeKind};
/// assert_eq!(detect_challenge("<html><body>hi</body></html>"), ChallengeKind::None);
/// ```
#[must_use]
pub fn detect_challenge(html: &str) -> ChallengeKind {
    if is_anubis(html) {
        ChallengeKind::Anubis
    } else if is_cloudflare(html) {
        ChallengeKind::Cloudflare
    } else {
        ChallengeKind::None
    }
}

/// Anubis markers. The challenge page sets a distinctive title and loads its
/// challenge script; the data attribute is specific enough to avoid collisions.
fn is_anubis(html: &str) -> bool {
    html.contains("Making sure you're not a bot!")
        || html.contains("/.within.website")
        || html.contains("data-challenge-state")
}

/// Cloudflare managed-challenge markers. "challenge-platform" and
/// "cf-chl-bypass" are Cloudflare-specific script/element identifiers.
fn is_cloudflare(html: &str) -> bool {
    html.contains("/cdn-cgi/challenge-platform/")
        || html.contains("cf-chl-bypass")
        || (html.contains("Just a moment...") && html.contains("challenge-platform"))
}

/// Polls for a challenge to clear, up to `timeout`.
///
/// `get_html` is called on each poll to fetch the current page HTML; the
/// function returns `Ok(())` once [`detect_challenge`] reports [`ChallengeKind::None`]
/// (i.e. the challenge solved and the real page loaded).
///
/// Taking a closure rather than a raw `Tab` keeps this loop unit-testable
/// without a real browser — the caller binds `|| tab.get_content().map_err(...)`.
///
/// # Errors
///
/// Returns [`FetchError::Render`] if the challenge does not clear within
/// `timeout`, or if `get_html` itself fails.
pub fn wait_for_clearance<F>(mut get_html: F, timeout: Duration) -> Result<(), FetchError>
where
    F: FnMut() -> Result<String, FetchError>,
{
    // Fast path: no challenge present, proceed immediately.
    if matches!(detect_challenge(&get_html()?), ChallengeKind::None) {
        return Ok(());
    }
    tracing::debug!("challenge detected; polling for clearance");

    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            return Err(FetchError::Render(
                "bot-protection challenge did not clear within timeout".to_owned(),
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
        if matches!(detect_challenge(&get_html()?), ChallengeKind::None) {
            tracing::debug!("challenge cleared");
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test assertions")]

    use super::*;

    #[test]
    fn clean_page_is_not_a_challenge() {
        // Given a normal page.
        let html = "<html><body><h1>Welcome</h1></body></html>";

        // When detecting.
        // Then no challenge is found.
        assert_eq!(detect_challenge(html), ChallengeKind::None);
    }

    #[test]
    fn anubis_title_marker_detected() {
        // Given a page with the Anubis challenge title.
        let html = r#"<html><head><title>Making sure you're not a bot!</title></head></html>"#;

        // When detecting.
        // Then it is an Anubis challenge.
        assert_eq!(detect_challenge(html), ChallengeKind::Anubis);
    }

    #[test]
    fn anubis_data_attribute_detected() {
        // Given a page with the Anubis challenge script data attribute.
        let html = r#"<div data-challenge-state="pending"></div>"#;

        // When detecting.
        // Then it is an Anubis challenge.
        assert_eq!(detect_challenge(html), ChallengeKind::Anubis);
    }

    #[test]
    fn cloudflare_challenge_platform_detected() {
        // Given a page loading the Cloudflare challenge platform script.
        let html = r#"<script src="/cdn-cgi/challenge-platform/h/g/orchestrate/jsch/v1"></script>"#;

        // When detecting.
        // Then it is a Cloudflare challenge.
        assert_eq!(detect_challenge(html), ChallengeKind::Cloudflare);
    }

    #[test]
    fn cloudflare_just_a_moment_without_platform_not_detected() {
        // Given a page that mentions "Just a moment..." but has no
        // challenge-platform script (avoids false positives).
        let html = "<p>Just a moment... I need to think.</p>";

        // When detecting.
        // Then no challenge is flagged (the conjunction is required).
        assert_eq!(detect_challenge(html), ChallengeKind::None);
    }

    #[test]
    fn wait_returns_immediately_when_no_challenge() {
        // Given a get_html closure that always returns clean content.
        let get_html = || Ok("<html><body>real content</body></html>".to_owned());

        // When waiting for clearance.
        let result = wait_for_clearance(get_html, Duration::from_secs(5));

        // Then it returns Ok immediately.
        assert!(result.is_ok());
    }

    #[test]
    fn wait_succeeds_when_challenge_clears() {
        // Given a closure that returns challenge HTML twice, then clean content.
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let get_html = || {
            let n = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < 2 {
                Ok(r#"<title>Making sure you're not a bot!</title>"#.to_owned())
            } else {
                Ok("<html><body>real content</body></html>".to_owned())
            }
        };

        // When waiting for clearance.
        let result = wait_for_clearance(get_html, Duration::from_secs(5));

        // Then it returns Ok once the challenge clears.
        assert!(result.is_ok());
    }

    #[test]
    fn wait_times_out_when_challenge_persists() {
        // Given a closure that always returns challenge HTML.
        let get_html = || Ok(r#"<script src="/cdn-cgi/challenge-platform/x"></script>"#.to_owned());

        // When waiting with a short timeout.
        let result = wait_for_clearance(get_html, Duration::from_millis(100));

        // Then it returns a Render error (no silent hang).
        assert!(matches!(result, Err(FetchError::Render(_))));
    }

    #[test]
    fn wait_propagates_get_html_error() {
        // Given a closure that fails.
        let get_html = || Err(FetchError::Render("tab died".to_owned()));

        // When waiting.
        let result = wait_for_clearance(get_html, Duration::from_secs(5));

        // Then the error propagates.
        assert!(matches!(result, Err(FetchError::Render(_))));
    }
}
