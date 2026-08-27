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

use serde::{Deserialize, Serialize};

use crate::FetchError;

/// The kind of bot-protection interstitial detected on a page, if any.
///
/// Serde round-trips so a detected kind can cross the actor bus inside
/// [`crate::FetchError::Challenge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChallengeKind {
    /// Anubis proof-of-work challenge page.
    Anubis,
    /// Cloudflare "Just a moment..." / Turnstile challenge page.
    Cloudflare,
    /// DuckDuckGo's own anti-bot / unusual-traffic anomaly page.
    ///
    /// Markers shared with the HTTP searcher (formerly its private
    /// `BLOCK_MARKERS`): `anomaly.js` and "If this error persists".
    DdgAnomaly,
    /// DataDome captcha interstitial (script host `js.datadome.co`).
    Datadome,
    /// PerimeterX / HUMAN captcha interstitial (`px-captcha` element).
    PerimeterX,
    /// Kasada challenge interstitial.
    Kasada,
    /// Imperva / Incapsula challenge interstitial.
    Imperva,
    /// No challenge detected — the page is real content.
    None,
}

impl ChallengeKind {
    /// Whether a real browser solves this challenge automatically.
    ///
    /// Anubis (proof-of-work) and Cloudflare (managed challenge/Turnstile)
    /// clear on their own once JavaScript runs, so the wait loop gives them
    /// an auto-clear window in both headless and headed modes. The others
    /// present a hard captcha a human must solve — waiting for them to
    /// auto-clear in headless mode would be pure added latency.
    #[must_use]
    pub const fn auto_clearable(self) -> bool {
        matches!(self, Self::Anubis | Self::Cloudflare)
    }
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
    } else if is_ddg_anomaly(html) {
        ChallengeKind::DdgAnomaly
    } else if is_datadome(html) {
        ChallengeKind::Datadome
    } else if is_perimeterx(html) {
        ChallengeKind::PerimeterX
    } else if is_kasada(html) {
        ChallengeKind::Kasada
    } else if is_imperva(html) {
        ChallengeKind::Imperva
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

/// Phrases that appear in DuckDuckGo's anti-bot / unusual-traffic anomaly page.
///
/// Moved from the HTTP searcher's private `BLOCK_MARKERS` so both backends
/// (reqwest and browser) detect the same page via [`detect_challenge`].
pub(crate) const DDG_BLOCK_MARKERS: [&str; 2] = ["anomaly.js", "If this error persists"];

/// DuckDuckGo anomaly markers.
fn is_ddg_anomaly(html: &str) -> bool {
    DDG_BLOCK_MARKERS.iter().any(|marker| html.contains(marker))
}

/// Returns `true` when the HTML is DuckDuckGo's anti-bot anomaly page.
///
/// Shared vocabulary for the HTTP searcher's post-response block check; the
/// browser path reaches the same verdict via [`detect_challenge`].
#[must_use]
pub fn is_ddg_blocked(html: &str) -> bool {
    is_ddg_anomaly(html)
}

/// DataDome markers: the challenge page loads its script from `js.datadome.co`.
fn is_datadome(html: &str) -> bool {
    html.contains("datadome")
}

/// PerimeterX / HUMAN markers: the captcha element id `px-captcha` and the
/// vendor's own script paths.
fn is_perimeterx(html: &str) -> bool {
    html.contains("px-captcha") || html.contains("perimeterx")
}

/// Kasada markers: the vendor's obfuscated challenge script path.
fn is_kasada(html: &str) -> bool {
    html.contains("kasada")
}

/// Imperva markers: the legacy Incapsula script identifier and the vendor name.
fn is_imperva(html: &str) -> bool {
    html.to_ascii_lowercase().contains("incapsula") || html.contains("imperva")
}

/// Visible-text threshold below which a page is suspected to be an
/// interstitial. Real content pages almost always carry far more than 200
/// characters of visible text; challenge shells render almost none.
const INTERSTITIAL_TEXT_THRESHOLD: usize = 200;

/// Measures the approximate visible text length of an HTML document.
///
/// Strips tags by splitting on `<` and keeping only what follows the
/// closing `>` of each segment (text nodes). No DOM parsing, no grapheme
/// indexing — a cheap density heuristic, not a correct HTML text extractor.
#[must_use]
pub fn visible_text_len(html: &str) -> usize {
    html.split('<').fold(0, |acc, segment| {
        let text = segment.split_once('>').map_or("", |(_, after)| after);
        acc + text.trim().len()
    })
}

/// Behavioral fallback: a page with near-zero visible text after render is
/// almost certainly an interstitial (an unknown vendor's challenge shell or
/// a hard block). This catches vendors the signature list doesn't know
/// without playing whack-a-mole.
#[must_use]
pub fn looks_like_interstitial(html: &str) -> bool {
    visible_text_len(html) < INTERSTITIAL_TEXT_THRESHOLD
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

// ---------------------------------------------------------------------------
// Tiered wait
// ---------------------------------------------------------------------------

/// How often the human-solve wait emits a [`RenderProgress::WaitingForHuman`]
/// tick. The ticks double as chat-log activity for the user and as history
/// activity for the stall watchdog.
const HUMAN_TICK_INTERVAL: Duration = Duration::from_secs(10);

/// Shared observer type for render progress notifications.
///
/// An `Arc`-shared closure so it can be cloned into blocking browser
/// tasks; `()` (a no-op) is used when no observer is needed.
pub type ProgressFn = std::sync::Arc<dyn Fn(RenderProgress) + Send + Sync>;

/// Tiered wait configuration, resolved from `[browser]` settings.
#[derive(Debug, Clone, Copy)]
pub struct WaitConfig {
    /// Window for auto-clearable challenges (Anubis PoW, Cloudflare managed)
    /// to solve themselves via JavaScript. Matches `anubis_timeout`.
    pub auto_timeout: Duration,
    /// Window a human has to solve a challenge in headed mode before the
    /// render gives up. Matches `[browser] challenge_wait_secs`.
    pub human_timeout: Duration,
    /// Settle window before the behavioral fallback renders a verdict on an
    /// empty-looking page (slow SPAs get a chance to fill). Matches
    /// `[browser] settle_secs`.
    pub settle: Duration,
    /// Whether the browser window is visible (a human can solve challenges).
    pub headed: bool,
}

/// Progress notification emitted by [`wait_for_content`] while waiting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderProgress {
    /// A challenge was detected and the wait escalated to a human solve.
    ChallengeDetected {
        /// Which challenge was detected.
        kind: ChallengeKind,
        /// The URL being rendered when the challenge appeared.
        url: String,
    },
    /// Periodic tick while waiting for a human to solve the challenge.
    WaitingForHuman {
        /// Seconds elapsed since the human wait began.
        elapsed_secs: u64,
    },
}

/// Verdict of the tiered wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    /// The page settled into real content; the caller may read it.
    Cleared,
    /// The page is (still) a challenge the caller must surface as an error.
    Challenge(ChallengeKind),
}

/// Waits for a rendered page to become real content, using a tiered strategy.
///
/// Tiers, in order:
///
/// 1. **Fast path** — no signature and healthy text density → `Cleared`
///    immediately (zero added delay).
/// 2. **Auto-clear window** — an auto-clearable signature (Anubis, Cloudflare)
///    gets `auto_timeout` to solve itself, in both modes.
/// 3. **Settle window** — an empty-looking page with no known signature gets
///    `settle` for a slow SPA to fill, then re-checks both signals.
/// 4. **Human window** (headed only) — an uncleared challenge keeps polling
///    up to `human_timeout`, emitting [`RenderProgress`] events so the user
///    knows to solve it in the visible browser tab. Headless returns the
///    `Challenge` verdict immediately instead — autonomous runs must not pay
///    a human-scale delay.
///
/// `on_event` receives every [`RenderProgress`]; callers relay it to the UI.
///
/// # Errors
///
/// Returns [`FetchError::Render`] when `get_html` itself fails (tab death).
pub fn wait_for_content<F>(
    mut get_html: F,
    cfg: &WaitConfig,
    url: &str,
    on_event: &dyn Fn(RenderProgress),
) -> Result<WaitOutcome, FetchError>
where
    F: FnMut() -> Result<String, FetchError>,
{
    let first = get_html()?;
    let signature = detect_challenge(&first);

    // Tier 1: fast path — healthy page, zero added delay.
    if signature == ChallengeKind::None && !looks_like_interstitial(&first) {
        return Ok(WaitOutcome::Cleared);
    }

    match signature {
        kind if kind.auto_clearable() => {
            wait_auto_then_escalate(&mut get_html, cfg, url, kind, on_event)
        }
        // A hard-captcha signature never auto-solves: escalate immediately
        // (headless returns the verdict without waiting).
        ChallengeKind::None => wait_settle_then_escalate(&mut get_html, cfg, url, on_event),
        kind => escalate(&mut get_html, cfg, url, kind, on_event),
    }
}

/// Tier 2: poll for an auto-clearable challenge to solve within `auto_timeout`.
fn wait_auto_then_escalate<F>(
    get_html: &mut F,
    cfg: &WaitConfig,
    url: &str,
    kind: ChallengeKind,
    on_event: &dyn Fn(RenderProgress),
) -> Result<WaitOutcome, FetchError>
where
    F: FnMut() -> Result<String, FetchError>,
{
    if poll_until_clear(get_html, cfg.auto_timeout)? {
        return Ok(WaitOutcome::Cleared);
    }
    escalate(get_html, cfg, url, kind, on_event)
}

/// Tier 3: give an empty-looking page one settle window to fill, then re-check
/// both the signature list and the density heuristic before escalating.
fn wait_settle_then_escalate<F>(
    get_html: &mut F,
    cfg: &WaitConfig,
    url: &str,
    on_event: &dyn Fn(RenderProgress),
) -> Result<WaitOutcome, FetchError>
where
    F: FnMut() -> Result<String, FetchError>,
{
    std::thread::sleep(cfg.settle);
    let html = get_html()?;
    let signature = detect_challenge(&html);
    if signature == ChallengeKind::None && !looks_like_interstitial(&html) {
        return Ok(WaitOutcome::Cleared);
    }
    // Still empty or now signature-matched: treat as an unknown challenge.
    escalate(get_html, cfg, url, signature, on_event)
}

/// Tier 4: surface the challenge to a human (headed) or fail fast (headless).
fn escalate<F>(
    get_html: &mut F,
    cfg: &WaitConfig,
    url: &str,
    kind: ChallengeKind,
    on_event: &dyn Fn(RenderProgress),
) -> Result<WaitOutcome, FetchError>
where
    F: FnMut() -> Result<String, FetchError>,
{
    if !cfg.headed {
        tracing::warn!(?kind, "challenge detected in headless mode; failing fast");
        return Ok(WaitOutcome::Challenge(kind));
    }
    on_event(RenderProgress::ChallengeDetected {
        kind,
        url: url.to_owned(),
    });
    let human_deadline = Instant::now() + cfg.human_timeout;
    let started = Instant::now();
    let mut next_tick = Instant::now() + HUMAN_TICK_INTERVAL;
    loop {
        if Instant::now() >= human_deadline {
            tracing::warn!(?kind, "challenge did not clear within human window");
            return Ok(WaitOutcome::Challenge(kind));
        }
        std::thread::sleep(POLL_INTERVAL);
        let html = get_html()?;
        if detect_challenge(&html) == ChallengeKind::None && !looks_like_interstitial(&html) {
            tracing::info!("challenge cleared after human window");
            return Ok(WaitOutcome::Cleared);
        }
        let now = Instant::now();
        if now >= next_tick {
            next_tick = now + HUMAN_TICK_INTERVAL;
            on_event(RenderProgress::WaitingForHuman {
                elapsed_secs: started.elapsed().as_secs(),
            });
        }
    }
}

/// Polls `get_html` every [`POLL_INTERVAL`] until the page is real content or
/// `timeout` elapses. Returns whether the page cleared.
fn poll_until_clear<F>(get_html: &mut F, timeout: Duration) -> Result<bool, FetchError>
where
    F: FnMut() -> Result<String, FetchError>,
{
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(POLL_INTERVAL);
        let html = get_html()?;
        if detect_challenge(&html) == ChallengeKind::None && !looks_like_interstitial(&html) {
            return Ok(true);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test assertions")]

    use super::*;

    #[rstest::rstest]
    #[test]
    fn clean_page_is_not_a_challenge() {
        // Given a normal page.
        let html = "<html><body><h1>Welcome</h1></body></html>";

        // When detecting.
        // Then no challenge is found.
        assert_eq!(detect_challenge(html), ChallengeKind::None);
    }

    #[rstest::rstest]
    #[test]
    fn anubis_title_marker_detected() {
        // Given a page with the Anubis challenge title.
        let html = r#"<html><head><title>Making sure you're not a bot!</title></head></html>"#;

        // When detecting.
        // Then it is an Anubis challenge.
        assert_eq!(detect_challenge(html), ChallengeKind::Anubis);
    }

    #[rstest::rstest]
    #[test]
    fn anubis_data_attribute_detected() {
        // Given a page with the Anubis challenge script data attribute.
        let html = r#"<div data-challenge-state="pending"></div>"#;

        // When detecting.
        // Then it is an Anubis challenge.
        assert_eq!(detect_challenge(html), ChallengeKind::Anubis);
    }

    #[rstest::rstest]
    #[test]
    fn cloudflare_challenge_platform_detected() {
        // Given a page loading the Cloudflare challenge platform script.
        let html = r#"<script src="/cdn-cgi/challenge-platform/h/g/orchestrate/jsch/v1"></script>"#;

        // When detecting.
        // Then it is a Cloudflare challenge.
        assert_eq!(detect_challenge(html), ChallengeKind::Cloudflare);
    }

    #[rstest::rstest]
    #[test]
    fn cloudflare_just_a_moment_without_platform_not_detected() {
        // Given a page that mentions "Just a moment..." but has no
        // challenge-platform script (avoids false positives).
        let html = "<p>Just a moment... I need to think.</p>";

        // When detecting.
        // Then no challenge is flagged (the conjunction is required).
        assert_eq!(detect_challenge(html), ChallengeKind::None);
    }

    #[rstest::rstest]
    #[test]
    fn wait_returns_immediately_when_no_challenge() {
        // Given a get_html closure that always returns clean content.
        let get_html = || Ok("<html><body>real content</body></html>".to_owned());

        // When waiting for clearance.
        let result = wait_for_clearance(get_html, Duration::from_secs(5));

        // Then it returns Ok immediately.
        assert!(result.is_ok());
    }

    #[rstest::rstest]
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

    #[rstest::rstest]
    #[test]
    fn wait_times_out_when_challenge_persists() {
        // Given a closure that always returns challenge HTML.
        let get_html = || Ok(r#"<script src="/cdn-cgi/challenge-platform/x"></script>"#.to_owned());

        // When waiting with a short timeout.
        let result = wait_for_clearance(get_html, Duration::from_millis(100));

        // Then it returns a Render error (no silent hang).
        assert!(matches!(result, Err(FetchError::Render(_))));
    }

    #[rstest::rstest]
    #[test]
    fn wait_propagates_get_html_error() {
        // Given a closure that fails.
        let get_html = || Err(FetchError::Render("tab died".to_owned()));

        // When waiting.
        let result = wait_for_clearance(get_html, Duration::from_secs(5));

        // Then the error propagates.
        assert!(matches!(result, Err(FetchError::Render(_))));
    }

    // -----------------------------------------------------------------------
    // Vendor signature tests
    // -----------------------------------------------------------------------

    #[rstest::rstest]
    #[test]
    fn ddg_anomaly_markers_detected() {
        // Given a page carrying DuckDuckGo's anomaly markers.
        let html = r#"<html><head><script src="/anomaly.js"></script></head>
            <body>If this error persists – please let us know</body></html>"#;

        // When detecting.
        // Then it is the DDG anomaly page.
        assert_eq!(detect_challenge(html), ChallengeKind::DdgAnomaly);
    }

    #[rstest::rstest]
    #[test]
    fn datadome_marker_detected() {
        // Given a page loading the DataDome challenge script.
        let html = r#"<script src="https://js.datadome.co/tags.js"></script>"#;

        // When detecting.
        // Then it is a DataDome challenge.
        assert_eq!(detect_challenge(html), ChallengeKind::Datadome);
    }

    #[rstest::rstest]
    #[test]
    fn perimeterx_marker_detected() {
        // Given a page with the PerimeterX captcha element.
        let html = r#"<div id="px-captcha"></div>"#;

        // When detecting.
        // Then it is a PerimeterX challenge.
        assert_eq!(detect_challenge(html), ChallengeKind::PerimeterX);
    }

    #[rstest::rstest]
    #[test]
    fn kasada_marker_detected() {
        // Given a page loading a Kasada challenge script.
        let html = r#"<script src="/_api/kasada/script"></script>"#;

        // When detecting.
        // Then it is a Kasada challenge.
        assert_eq!(detect_challenge(html), ChallengeKind::Kasada);
    }

    #[rstest::rstest]
    #[test]
    fn imperva_marker_detected() {
        // Given a page with the Incapsula script identifier.
        let html = r#"<script src="/_Incapsula_Resource"></script>"#;

        // When detecting.
        // Then it is an Imperva challenge.
        assert_eq!(detect_challenge(html), ChallengeKind::Imperva);
    }

    #[rstest::rstest]
    #[test]
    fn is_ddg_blocked_matches_detection() {
        // Given the DDG anomaly page.
        let html = r#"<script src="/anomaly.js"></script>"#;

        // When asking the shared helper.
        // Then it agrees with detect_challenge.
        assert!(is_ddg_blocked(html));
        assert_eq!(detect_challenge(html), ChallengeKind::DdgAnomaly);
    }

    #[rstest::rstest]
    #[case(ChallengeKind::Anubis, true)]
    #[case(ChallengeKind::Cloudflare, true)]
    #[case(ChallengeKind::DdgAnomaly, false)]
    #[case(ChallengeKind::Datadome, false)]
    #[case(ChallengeKind::PerimeterX, false)]
    #[case(ChallengeKind::Kasada, false)]
    #[case(ChallengeKind::Imperva, false)]
    fn auto_clearable_matches_vendor_behavior(#[case] kind: ChallengeKind, #[case] expected: bool) {
        // Given a challenge kind.
        // When asking whether it auto-clears.
        // Then only Anubis and Cloudflare do.
        assert_eq!(kind.auto_clearable(), expected);
    }

    // -----------------------------------------------------------------------
    // Behavioral fallback tests
    // -----------------------------------------------------------------------

    #[rstest::rstest]
    #[test]
    fn full_article_page_is_not_interstitial() {
        // Given a page with substantial visible text.
        let html = format!(
            "<html><body><p>{}</p></body></html>",
            "lorem ipsum dolor sit amet. ".repeat(50)
        );

        // When checking the density heuristic.
        // Then it is not suspected.
        assert!(!looks_like_interstitial(&html));
    }

    #[rstest::rstest]
    #[test]
    fn near_empty_shell_is_interstitial_suspected() {
        // Given an interstitial shell: big head, almost no visible text.
        let html = "<html><head><meta charset=utf-8><style>/* big css */</style></head>\
            <body><noscript>JS required</noscript></body></html>";

        // When checking the density heuristic.
        // Then it is suspected.
        assert!(looks_like_interstitial(html));
    }

    #[rstest::rstest]
    #[test]
    fn visible_text_len_strips_tags() {
        // Given markup with tags and text nodes.
        let html = "<p>hello</p><div> world </div>";

        // When measuring visible text.
        // Then only the trimmed text nodes count.
        assert_eq!(visible_text_len(html), "hello".len() + "world".len());
    }

    // -----------------------------------------------------------------------
    // Tiered wait tests
    // -----------------------------------------------------------------------

    fn fast_cfg() -> WaitConfig {
        WaitConfig {
            auto_timeout: Duration::from_millis(300),
            human_timeout: Duration::from_millis(300),
            settle: Duration::from_millis(50),
            headed: false,
        }
    }

    fn healthy_page() -> String {
        format!(
            "<html><body><p>{}</p></body></html>",
            "real content here. ".repeat(50)
        )
    }

    #[rstest::rstest]
    #[test]
    fn healthy_page_clears_immediately_with_one_fetch() {
        // Given a closure serving a healthy page and counting calls.
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let get_html = || {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(healthy_page())
        };

        // When waiting with any config.
        let outcome = wait_for_content(get_html, &fast_cfg(), "https://x", &|_| {}).expect("ok");

        // Then it cleared with exactly one fetch — zero added delay.
        assert_eq!(outcome, WaitOutcome::Cleared);
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "fast path must not poll a healthy page"
        );
    }

    #[rstest::rstest]
    #[test]
    fn hard_signature_headless_returns_challenge_without_waiting() {
        // Given a closure always serving the DDG anomaly page (not auto-clearable).
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let get_html = || {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(String::from(r#"<script src="/anomaly.js"></script>"#))
        };

        // When waiting headless.
        let started = Instant::now();
        let outcome = wait_for_content(get_html, &fast_cfg(), "https://x", &|_| {}).expect("ok");
        let elapsed = started.elapsed();

        // Then the Challenge verdict is immediate (no auto window, no human window).
        assert_eq!(outcome, WaitOutcome::Challenge(ChallengeKind::DdgAnomaly));
        assert!(
            elapsed < Duration::from_secs(2),
            "headless must fail fast, took {elapsed:?}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn auto_clearable_challenge_headless_gets_auto_window_then_verdict() {
        // Given a closure serving a Cloudflare page that never clears.
        let get_html = || {
            Ok(String::from(
                r#"<script src="/cdn-cgi/challenge-platform/h/g/orchestrate"></script>"#,
            ))
        };

        // When waiting headless with a 300ms auto window.
        let outcome = wait_for_content(get_html, &fast_cfg(), "https://x", &|_| {}).expect("ok");

        // Then the auto window expired and the verdict is a Cloudflare challenge.
        assert_eq!(outcome, WaitOutcome::Challenge(ChallengeKind::Cloudflare));
    }

    #[rstest::rstest]
    #[test]
    fn auto_clearable_challenge_that_solves_returns_cleared() {
        // Given a closure serving one Cloudflare page then real content.
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let get_html = || {
            let n = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                Ok(String::from(
                    r#"<script src="/cdn-cgi/challenge-platform/h/g/orchestrate"></script>"#,
                ))
            } else {
                Ok(healthy_page())
            }
        };

        // When waiting headless.
        let outcome = wait_for_content(get_html, &fast_cfg(), "https://x", &|_| {}).expect("ok");

        // Then the challenge auto-cleared.
        assert_eq!(outcome, WaitOutcome::Cleared);
    }

    #[rstest::rstest]
    #[test]
    fn empty_page_headless_verdict_after_settle() {
        // Given a closure always serving an empty shell with no signature.
        let get_html = || Ok(String::from("<html><body></body></html>"));

        // When waiting headless with a short settle.
        let started = Instant::now();
        let outcome = wait_for_content(get_html, &fast_cfg(), "https://x", &|_| {}).expect("ok");
        let elapsed = started.elapsed();

        // Then the verdict is an unknown challenge, after roughly the settle window.
        assert_eq!(outcome, WaitOutcome::Challenge(ChallengeKind::None));
        assert!(
            elapsed >= Duration::from_millis(50),
            "settle window must elapse"
        );
    }

    #[rstest::rstest]
    #[test]
    fn slow_spa_filling_within_settle_clears() {
        // Given a closure serving an empty shell first, then real content.
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let get_html = || {
            let n = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                Ok(String::from("<html><body></body></html>"))
            } else {
                Ok(healthy_page())
            }
        };

        // When waiting headless.
        let outcome = wait_for_content(get_html, &fast_cfg(), "https://x", &|_| {}).expect("ok");

        // Then the page filled within the settle window and cleared.
        assert_eq!(outcome, WaitOutcome::Cleared);
    }

    #[rstest::rstest]
    #[test]
    fn headed_escalation_emits_detection_and_ticks_then_verdict() {
        // Given a closure always serving the DDG anomaly page, headed mode.
        let cfg = WaitConfig {
            headed: true,
            ..fast_cfg()
        };
        let get_html = || Ok(String::from(r#"<script src="/anomaly.js"></script>"#));
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = {
            let events = std::sync::Arc::clone(&events);
            move |e: RenderProgress| events.lock().expect("lock").push(e)
        };

        // When waiting with a 300ms human window (shorter than a tick, so no
        // WaitingForHuman tick fires before the deadline).
        let outcome = wait_for_content(get_html, &cfg, "https://x", &sink).expect("ok");

        // Then the detection event fired and the verdict is the challenge.
        assert_eq!(outcome, WaitOutcome::Challenge(ChallengeKind::DdgAnomaly));
        let events = events.lock().expect("lock");
        assert!(
            events.iter().any(|e| matches!(
                e,
                RenderProgress::ChallengeDetected {
                    kind: ChallengeKind::DdgAnomaly,
                    ..
                }
            )),
            "ChallengeDetected must be emitted, got {events:?}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn headed_challenge_cleared_mid_window_returns_cleared() {
        // Given a closure serving the anomaly page once, then real content.
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let get_html = || {
            let n = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                Ok(String::from(r#"<script src="/anomaly.js"></script>"#))
            } else {
                Ok(healthy_page())
            }
        };
        // And a headed config whose human window covers one poll interval.
        let cfg = WaitConfig {
            human_timeout: Duration::from_secs(10),
            headed: true,
            ..fast_cfg()
        };

        // When waiting.
        let outcome = wait_for_content(get_html, &cfg, "https://x", &|_| {}).expect("ok");

        // Then the human-solved challenge cleared within the window.
        assert_eq!(outcome, WaitOutcome::Cleared);
    }
}
