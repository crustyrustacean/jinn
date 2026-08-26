//! Shared browser — a lazily-launched, self-healing Chromium process shared
//! across multiple consumers (fetcher + searcher).
//!
//! A [`SharedBrowser`] owns one browser process behind a mutex-guarded slot.
//! Consumers call [`SharedBrowser::render_page`] to open a tab, navigate, wait
//! for interstitial clearance, and return the rendered HTML. Each call opens
//! its own tab, so concurrent consumers run independent tabs off the same
//! process — exactly the pattern that makes one warmed profile serve both the
//! `web-fetch` and `web-search` tools.
//!
//! # Crash recovery
//!
//! If a tab operation fails with a connection-level death (idle-teardown,
//! browser crash, OOM kill), the browser handle is cleared from the internal
//! [`Mutex`] and the render is retried exactly once against a freshly-launched
//! browser. Per-tab failures (a bad page, a tab-level timeout) do _not_ evict
//! the shared browser, since under concurrency that would kill other consumers'
//! in-flight tabs.
//!
//! # Heartbeat health
//!
//! Crash recovery above is _reactive_ — it only fires when a render actually
//! hits the dead socket. Long-running sessions need a _proactive_ liveness
//! check so a wedged browser is noticed even while no render is in flight.
//! A detached heartbeat task ([`HEARTBEAT_INTERVAL`]) calls [`SharedBrowser::probe`],
//! which runs [`HeadlessBrowser::liveness`] (a cheap `Browser::get_version`)
//! inside a [`PROBE_TIMEOUT`]-bounded blocking task. A probe failure, timeout,
//! or panic force-evicts the handle via [`SharedBrowser::force_evict`]; the next
//! request then lazily launches a fresh browser on the normal render path. A
//! healthy browser is never torn down by the heartbeat — a successful probe
//! counts as incoming transport traffic, resetting the library's idle timer.
//! The two mechanisms compose: the heartbeat catches idle death proactively,
//! crash recovery catches in-flight death reactively.
//!
//! # Lifecycle
//!
//! - **Lazy launch**: first `render_page` starts Chromium.
//! - **Reuse**: subsequent calls open a new tab on the same browser.
//! - **Self-heal**: a dead WebSocket triggers exactly one relaunch + retry.
//! - **Heartbeat**: a periodic probe force-evicts a wedged browser; the next
//!   request lazily relaunches.
//! - **Shutdown**: [`SharedBrowser::shutdown`] drops the browser (kills process).
//!
//! # SingletonLock
//!
//! At most one Chromium process may own a given `--user-data-dir` (profile
//! dir). [`SharedBrowser`] honors this by holding a single process; all
//! consumers on the same mode (headless or headed) share it.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Error as AnyhowError;
use headless_chrome::{Browser, LaunchOptions};
use parking_lot::Mutex;

use crate::FetchError;
use crate::stealth::StealthSettings;

/// A last-resort upper bound on how long a single in-flight render may wait
/// on the underlying connection before headless_chrome cancels it.
///
/// This is NOT the primary health signal — the heartbeat
/// ([`HEARTBEAT_INTERVAL`] + [`SharedBrowser::probe`]) owns proactive liveness and
/// force-evicts a dead browser within ~10s. The library sets this same value as
/// both the per-call wait AND the idle-teardown timer; it covers one narrow case
/// the heartbeat cannot: a render already in flight at the exact instant the
/// socket dies, holding its own handle that the heartbeat's eviction can't kill
/// out from under it. 60s caps that one render; every subsequent render recovers
/// immediately via the normal lazy-relaunch path. Never wait 10 minutes again.
// `Duration::from_mins` is unstable, so we express the constant in seconds.
#[expect(
    clippy::duration_suboptimal_units,
    reason = "`Duration::from_mins` is unstable; expressed in seconds"
)]
pub(crate) const IDLE_BROWSER_TIMEOUT: Duration = Duration::from_secs(60);

/// How often the keepalive heartbeat runs [`SharedBrowser::probe`].
///
/// On a healthy browser a successful probe response counts as incoming
/// transport traffic, resetting the library's idle timer — so a warmed
/// browser is never torn down merely for being idle. On a dead/wedged
/// socket, this is the worst-case window before the heartbeat notices and
/// force-evicts so the next request relaunches.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);

/// How long a single liveness probe may take before the heartbeat treats it
/// as a wedge and force-evicts.
///
/// A healthy `Browser::get_version` round-trip is well under a second; this
/// bound exists only to cut a stuck CDP call short (the orphaned blocking
/// thread is harmless — see [`SharedBrowser::probe`]).
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// A page rendered to HTML by a browser tab.
///
/// Extraction (text/markdown) is the consumer's concern, applied after render,
/// so the browser abstraction stays format-agnostic. This is what lets the
/// shared browser serve both the fetcher (which extracts) and the searcher
/// (which parses result HTML) without knowing either's format.
#[derive(Clone)]
pub struct RenderedPage {
    /// Raw HTML after JavaScript execution.
    pub html: String,
    /// Final URL after any redirects.
    pub final_url: String,
}

/// Capability: render one page to HTML in a browser tab.
///
/// Abstracts the concrete [`headless_chrome::Browser`] so the launch,
/// eviction, and retry logic is unit-testable without spawning Chromium.
/// Implementations classify their own errors: connection death surfaces as
/// [`FetchError::BrowserCrash`]; per-tab failures as [`FetchError::Render`].
pub trait HeadlessBrowser: Send + Sync {
    /// Renders `url` to a page, reporting wait progress through `on_progress`.
    ///
    /// `on_progress` receives [`RenderProgress`] events while the render
    /// waits on a challenge (detection, human-wait ticks). Implementations
    /// that never wait simply never call it.
    ///
    /// # Errors
    ///
    /// [`FetchError::BrowserCrash`] when the shared connection is dead;
    /// [`FetchError::Render`] for per-tab failures; [`FetchError::Challenge`]
    /// when a detected challenge did not clear.
    fn render(
        &self,
        url: &str,
        on_progress: &dyn Fn(crate::challenge::RenderProgress),
    ) -> Result<RenderedPage, FetchError>;

    /// Probes whether the browser is still alive.
    ///
    /// The heartbeat ([`SharedBrowser::probe`]) calls this periodically; a
    /// failure force-evicts the handle so the next request lazily launches
    /// a fresh browser. This is the proactive health check that defeats a
    /// dead/wedged WebSocket that the render path would otherwise only notice
    /// passively via the library's idle timer.
    ///
    /// # Errors
    ///
    /// [`FetchError::BrowserCrash`] when the shared connection is dead;
    /// [`FetchError::Render`] for per-call failures.
    fn liveness(&self) -> Result<(), FetchError>;
    /// Backend identifier for tracing/debug.
    fn name(&self) -> &'static str;
}

/// Capability: launch a fresh browser handle.
///
/// The shared browser calls this on first use and on every crash-recovery
/// relaunch. Under concurrency the slot mutex serializes relaunches, so only
/// one task actually launches and the rest reuse it.
pub trait HeadlessBrowserFactory: Send + Sync {
    /// Launches a new browser.
    ///
    /// # Errors
    ///
    /// [`FetchError::BrowserLaunch`] if the process cannot be started.
    fn launch(&self) -> Result<Arc<dyn HeadlessBrowser>, FetchError>;
    /// Factory identifier for tracing/debug.
    fn name(&self) -> &'static str;
}

/// The [`LaunchOptions`] used for every Chromium launch.
///
/// Stealth launch flags are added when `settings.enabled` is true. The binary
/// path from `settings.binary_path` is passed through so a system-installed
/// branded Chrome (preferred) or Chromium can be selected; when `None`, the
/// `headless_chrome` crate's own discovery is used.
///
/// When `settings.headed` is `true`, the browser launches a visible window
/// (`headless: false`) so the user can manually solve interstitial
/// challenges. When `settings.profile_dir` is `Some`, the profile is made
/// persistent via `--user-data-dir` so cookies/clearances survive restart.
///
/// Exposed `pub(crate)` so the idle-timeout, stealth-arg, headed, and
/// profile-dir invariants are unit-testable.
pub(crate) fn build_launch_options(settings: &StealthSettings) -> LaunchOptions<'static> {
    // `--disable-blink-features=AutomationControlled` is the primary tell
    // suppressor: it stops Chrome from setting navigator.webdriver and
    // advertizing automation. The site-isolation flags avoid a secondary
    // tell left by the default process model.
    let mut args: Vec<&'static std::ffi::OsStr> = Vec::new();
    if settings.enabled {
        args.push("--disable-blink-features=AutomationControlled".as_ref());
        args.push("--disable-features=IsolateOrigins,site-per-process".as_ref());
    }

    LaunchOptions {
        headless: !settings.headed,
        idle_browser_timeout: IDLE_BROWSER_TIMEOUT,
        path: settings.binary_path.clone(),
        user_data_dir: settings.profile_dir.clone(),
        args,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Concrete headless_chrome backend
// ---------------------------------------------------------------------------

/// Production backend: wraps a real [`headless_chrome::Browser`].
pub struct ChromeBrowser {
    browser: Browser,
    /// Stealth settings applied per-tab in `render`.
    stealth: StealthSettings,
}

impl HeadlessBrowser for ChromeBrowser {
    fn render(
        &self,
        url: &str,
        on_progress: &dyn Fn(crate::challenge::RenderProgress),
    ) -> Result<RenderedPage, FetchError> {
        let tab = self
            .browser
            .new_tab()
            .map_err(|e| classify_browser_error(&e))?;

        // Stealth: apply per-tab BEFORE navigation so the patches are in place
        // before any page script runs. Order matters — enable_stealth_mode()
        // sets a naive hardcoded UA via bypass_user_agent(); our explicit
        // set_user_agent() call AFTER it overrides that with the correct
        // OS-matched string and Accept-Language.
        if self.stealth.enabled {
            tracing::trace!("SharedBrowser: applying stealth mode");
            tab.enable_stealth_mode()
                .map_err(|e| classify_browser_error(&e))?;
            tab.set_user_agent(
                &self.stealth.user_agent,
                Some(&self.stealth.accept_language),
                Some(&self.stealth.platform),
            )
            .map_err(|e| classify_browser_error(&e))?;
        }

        tracing::trace!(url = %url, "SharedBrowser: navigating to URL");
        tab.navigate_to(url)
            .map_err(|e| classify_browser_error(&e))?
            .wait_until_navigated()
            .map_err(|e| classify_browser_error(&e))?;
        tracing::trace!("SharedBrowser: navigation complete");

        // Challenge-aware tiered wait: navigate returns when the interstitial
        // loads, not when the page is real content. The wait covers vendor
        // signatures (auto-clear + human windows) and the behavioral fallback.
        // The tab stays open for the entire wait — a human solving the
        // challenge needs it on screen.
        let cfg = crate::challenge::WaitConfig {
            auto_timeout: self.stealth.anubis_timeout,
            human_timeout: self.stealth.challenge_wait,
            settle: self.stealth.settle,
            headed: self.stealth.headed,
        };
        let get_html = || tab.get_content().map_err(|e| classify_browser_error(&e));
        match crate::challenge::wait_for_content(get_html, &cfg, url, on_progress)? {
            crate::challenge::WaitOutcome::Cleared => {}
            crate::challenge::WaitOutcome::Challenge(kind) => {
                // Nothing more to solve: close the tab and surface the verdict.
                close_tab_if_configured(&tab, self.stealth.keep_tabs_open);
                return Err(FetchError::Challenge { kind });
            }
        }

        tracing::trace!("SharedBrowser: getting page HTML");
        let html = tab.get_content().map_err(|e| classify_browser_error(&e))?;
        tracing::debug!(html_len = html.len(), "SharedBrowser: HTML retrieved");

        let final_url = tab.get_url();
        tracing::debug!(final_url = %final_url, "SharedBrowser: final URL");

        tracing::trace!("SharedBrowser: closing tab");
        close_tab_if_configured(&tab, self.stealth.keep_tabs_open);

        Ok(RenderedPage { html, final_url })
    }

    fn liveness(&self) -> Result<(), FetchError> {
        // get_version is the cheapest CDP round-trip; its success exercises
        // the same call_method -> util::Wait path the render uses, so a
        // effect. Only Ok/Err matters here — the payload is discarded.
        self.browser
            .get_version()
            .map_err(|e| classify_browser_error(&e))?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "headless_chrome::Browser"
    }
}

/// Closes `tab` unless the user asked to keep render tabs open.
///
/// The `[browser] keep_tabs_open` setting trades tab hygiene for
/// inspectability: open tabs let a human (or developer) inspect what the
/// browser actually rendered — useful when hunting new challenge pages.
fn close_tab_if_configured(tab: &headless_chrome::Tab, keep_open: bool) {
    if !keep_open {
        let _ = tab.close(true);
    }
}

/// Production factory: launches a real Chromium via [`headless_chrome`].
///
/// Carries the [`StealthSettings`] so every launch applies the configured
/// anti-detection flags and binary path.
pub struct ChromeFactory {
    stealth: StealthSettings,
}

impl ChromeFactory {
    /// Creates a factory that launches with the given stealth settings.
    #[must_use]
    pub fn new(stealth: StealthSettings) -> Self {
        Self { stealth }
    }
}

impl HeadlessBrowserFactory for ChromeFactory {
    fn launch(&self) -> Result<Arc<dyn HeadlessBrowser>, FetchError> {
        tracing::info!(
            stealth = self.stealth.enabled,
            headed = self.stealth.headed,
            "SharedBrowser: launching Chrome"
        );
        let stealth = self.stealth.clone();
        let browser = Browser::new(build_launch_options(&self.stealth)).map_err(|e| {
            tracing::error!(err = %e, "SharedBrowser: failed to launch browser");
            FetchError::BrowserLaunch
        })?;
        tracing::info!("SharedBrowser: browser launched successfully");
        Ok(Arc::new(ChromeBrowser { browser, stealth }))
    }

    fn name(&self) -> &'static str {
        "ChromeFactory"
    }
}

/// The type of a connection-level death in headless_chrome.
///
/// Re-exported from `headless_chrome::browser::transport`. Detecting this
/// *by type* (via [`classify_browser_error`]) is the primary, robust signal
/// for a dead shared WebSocket — it survives any future rewording of the
/// The string-marker fallback in [`is_connection_closed`] exists for
/// defense in depth.
use headless_chrome::browser::transport::ConnectionClosed;

/// The literal text of the headless_chrome `ConnectionClosed` error.
///
/// Defensive fallback for [`classify_browser_error`]: if a future
/// `headless_chrome` release changes how the error is surfaced such that
/// the type downcast misses, this substring still catches the (currently
/// stable) display message. See the crate's
/// `src/browser/transport/mod.rs`:
/// `#[error("Unable to make method calls because underlying connection is closed")]`.
const CONNECTION_CLOSED_MARKER: &str = "underlying connection is closed";

/// Maps a headless_chrome failure to a [`FetchError`].
///
/// Detection order:
/// 1. **Type downcast** (primary): if the error *is* (or wraps) a
///    [`ConnectionClosed`], classify as [`FetchError::BrowserCrash`]. This
///    is type-safe and does not depend on the error's `Display` text.
/// 2. **String match** (fallback): if the display string contains the
///    [`CONNECTION_CLOSED_MARKER`] substring, also classify as
///    [`FetchError::BrowserCrash`]. Guards against future surfacing changes.
/// 3. Otherwise, classify as [`FetchError::Render`].
///
/// `ConnectionClosed` (the shared WebSocket died: idle-teardown timeout, OOM
/// kill, real crash) must trigger eviction + relaunch; per-tab failures (a bad
/// page, a tab-level timeout) stay as [`FetchError::Render`] so they never
/// evict the shared browser under concurrency.
pub(crate) fn classify_browser_error(err: &AnyhowError) -> FetchError {
    if is_connection_closed(err) {
        FetchError::BrowserCrash
    } else {
        FetchError::Render(err.to_string())
    }
}

/// Returns `true` if `err` represents a connection-level death.
///
/// Primary signal is a type downcast; the string marker is a fallback.
/// The downcast walks the full error source chain, so a context-wrapped
/// `ConnectionClosed` (e.g. some future `headless_chrome` release that
/// starts using `.context(..)`) is still detected.
pub(crate) fn is_connection_closed(err: &AnyhowError) -> bool {
    // `chain()` includes the head error first, then each `.source()`.
    let type_match = err
        .chain()
        .any(|e| e.downcast_ref::<ConnectionClosed>().is_some());
    type_match || err.to_string().contains(CONNECTION_CLOSED_MARKER)
}

/// Maps a headless_chrome failure string to a [`FetchError`].
///
/// Test-only seam that drives [`is_connection_closed`] from a raw display
/// string. Production routes through [`classify_browser_error`] (type
/// downcast + string fallback). Exposed so the string-fallback path can be
/// unit-tested in isolation without constructing an `anyhow::Error`.
///
/// Detects `ConnectionClosed` (the shared WebSocket died: idle-teardown
/// timeout, OOM kill, real crash) via its error message and maps it to
/// [`FetchError::BrowserCrash`] so the caller can evict the shared browser
/// and relaunch. All other failures (a bad page, a tab-level timeout) stay
/// as [`FetchError::Render`], since they must not evict the shared browser
/// under concurrency.
#[cfg(test)]
pub(crate) fn classify_render_error(display: &str) -> FetchError {
    if display.contains(CONNECTION_CLOSED_MARKER) {
        FetchError::BrowserCrash
    } else {
        FetchError::Render(display.to_owned())
    }
}

// ---------------------------------------------------------------------------
// SharedBrowser
// ---------------------------------------------------------------------------

/// A shared, lazily-launched browser process that multiple consumers attach
/// to by opening tabs.
///
/// Owns one browser handle behind a mutex-guarded slot, held only during
/// launch/clone/evict — never across a render — so concurrent consumers run
/// independent tabs. Because launch happens under the lock, concurrent
/// crash-recovery attempts funnel to exactly one relaunch (no thundering
/// herd).
///
/// Construct once per browser mode (headless or headed) and share the
/// `Arc<SharedBrowser>` among all consumers on that mode. Each mode needs
/// its own `SharedBrowser` with its own profile dir (Chromium's
/// `SingletonLock` forbids two processes on one dir).
pub struct SharedBrowser {
    /// The lazily-launched browser instance.
    slot: Arc<Mutex<Option<Arc<dyn HeadlessBrowser>>>>,
    /// Produces new browser handles on first use and crash recovery.
    factory: Arc<dyn HeadlessBrowserFactory>,
}

impl SharedBrowser {
    /// Creates a shared browser that launches with the given stealth settings
    /// via the production [`ChromeFactory`]. The browser is not launched until
    /// the first [`Self::render_page`] call.
    #[must_use]
    pub fn new(stealth: StealthSettings) -> Self {
        Self::with_factory(Arc::new(ChromeFactory::new(stealth)))
    }

    /// Test seam: creates a shared browser backed by a swappable factory.
    ///
    /// Production code uses [`Self::new`] (the real Chromium factory). Tests
    /// inject a fake factory to drive self-heal, retry, and eviction behavior
    /// without spawning Chrome.
    #[must_use]
    pub fn with_factory(factory: Arc<dyn HeadlessBrowserFactory>) -> Self {
        Self {
            slot: Arc::new(Mutex::new(None)),
            factory,
        }
    }

    /// Renders `url` to a page, launching the browser on first use and
    /// retrying exactly once after a connection-level death.
    ///
    /// This is the unit of work both the fetcher and the searcher drive: open
    /// a tab, navigate, wait for clearance, return the raw HTML. Extraction
    /// / parsing is the caller's concern. Discards any wait progress — use
    /// [`Self::render_page_observed`] when the caller surfaces waits.
    ///
    /// # Errors
    ///
    /// [`FetchError::BrowserCrash`] if the browser dies and the relaunch
    /// attempt also dies; [`FetchError::Render`] for per-tab failures;
    /// [`FetchError::BrowserLaunch`] if the process cannot be started.
    pub fn render_page(&self, url: &str) -> Result<RenderedPage, FetchError> {
        self.render_page_observed(url, &(Arc::new(|_| {}) as crate::challenge::ProgressFn))
    }

    /// Renders `url` with wait progress relayed to `on_progress`.
    ///
    /// Same launch/retry semantics as [`Self::render_page`]; the observer
    /// receives [`RenderProgress`] events while a challenge wait runs
    /// (detection + human-wait ticks) so callers can surface "solve it in
    /// the browser window" to the user.
    ///
    /// # Errors
    ///
    /// As [`Self::render_page`], plus [`FetchError::Challenge`] when a
    /// detected challenge did not clear.
    pub fn render_page_observed(
        &self,
        url: &str,
        on_progress: &crate::challenge::ProgressFn,
    ) -> Result<RenderedPage, FetchError> {
        match self.render_once(url, &**on_progress) {
            // Connection-level death: render_once already evicted the handle;
            // relaunch and retry exactly once.
            Err(FetchError::BrowserCrash) => {
                tracing::info!("SharedBrowser: retrying after connection death");
                self.render_once(url, &**on_progress)
            }
            other => other,
        }
    }

    /// One render attempt against the cached browser: ensure a handle, render.
    ///
    /// On a connection-level death ([`FetchError::BrowserCrash`]), evicts the
    /// shared handle so the next attempt relaunches. Per-tab failures
    /// ([`FetchError::Render`]) are returned without eviction — evicting on
    /// them would kill other consumers' in-flight tabs under concurrency.
    fn render_once(
        &self,
        url: &str,
        on_progress: &dyn Fn(crate::challenge::RenderProgress),
    ) -> Result<RenderedPage, FetchError> {
        let browser = ensure_browser(&self.slot, &self.factory)?;
        match browser.render(url, on_progress) {
            Ok(page) => Ok(page),
            Err(err) => {
                tracing::warn!(err = %err, "SharedBrowser: render failed");
                // Evict only on connection death, and only if the slot still
                // holds THIS task's handle — a concurrent task may have
                // already relaunched.
                if matches!(err, FetchError::BrowserCrash | FetchError::BrowserLaunch)
                    && evict_if_matching(&self.slot, &browser)
                {
                    tracing::info!("SharedBrowser: clearing browser for crash recovery");
                }
                Err(err)
            }
        }
    }

    /// Force-evicts the cached browser handle, killing the Chromium process.
    ///
    /// Used by the heartbeat ([`Self::probe`]) on a failed liveness check.
    /// Unconditional — unlike the render path's identity-scoped [`evict_if_matching`],
    /// the heartbeat has no offender handle to compare against and must clear
    /// whatever is in the slot. Idempotent: a no-op when the slot is already empty.
    ///
    /// The next request after an eviction lazily launches a fresh browser via
    /// [`ensure_browser`] on the normal render path — no launch logic lives here.
    pub fn force_evict(&self) {
        // Take the handle out of the slot under a `let` binding so the guard is
        // released at the `;` — NOT held via if-let temporary extension. The
        // browser drop (which kills the Chromium process) then happens entirely
        // outside the lock, so a slow teardown can never stall concurrent probes/renders.
        let evicted = self.slot.lock().take();
        if let Some(browser) = evicted {
            tracing::info!(
                backend = browser.name(),
                "SharedBrowser: force-evicting browser (kills Chromium process)"
            );
            drop(browser);
        } else {
            tracing::trace!("SharedBrowser: force-evict was a no-op (slot empty)");
        }
    }

    /// Probes whether the cached browser is still alive, evicting it if not.
    ///
    /// This is the heartbeat entry point: cheap, never launches, and idempotent.
    /// On an empty slot it returns immediately (preserves lazy launch — the
    /// heartbeat must never be the reason a browser exists). On a filled slot it
    /// runs [`HeadlessBrowser::liveness`]; on any failure it calls
    /// [`Self::force_evict`] so the next request lazily launches a fresh browser.
    pub fn probe(&self) {
        // Snapshot the handle (if any) WITHOUT launching. Cloning the Arc
        // under the lock, then releasing, keeps the lock hold minimal.
        let Some(browser) = self.slot.lock().clone() else {
            tracing::trace!("SharedBrowser: probe skipped (no browser)");
            return;
        };
        match browser.liveness() {
            Ok(()) => tracing::trace!("SharedBrowser: probe ok"),
            Err(err) => {
                tracing::warn!(err = %err, "SharedBrowser: probe failed, evicting");
                self.force_evict();
            }
        }
    }
    /// Drops the cached browser handle, killing the Chromium process.
    ///
    /// Called during application shutdown. Safe to call multiple times.
    #[expect(
        clippy::unused_async,
        reason = "async for shutdown-completion symmetry"
    )]
    pub async fn shutdown(&self) {
        tracing::info!("SharedBrowser: shutting down");
        // Take under a `let` so the guard releases at the `;`; the browser drop
        // (which kills Chromium) runs outside the lock.
        let evicted = self.slot.lock().take();
        if let Some(browser) = evicted {
            tracing::debug!(
                backend = browser.name(),
                "SharedBrowser: dropping browser (kills Chromium process)"
            );
            drop(browser);
        }
        tracing::info!("SharedBrowser: shutdown complete");
    }
}

/// Ensures a browser is running in `slot`, launching one if necessary.
///
/// Returns a clone of the handle. The lock is held only long enough to
/// check/launch/clone — never across a render — so concurrent consumers can
/// each grab a handle and run independent tabs. Because launch happens under
/// the lock, concurrent crash-recovery attempts funnel to exactly one relaunch.
fn ensure_browser(
    slot: &Arc<Mutex<Option<Arc<dyn HeadlessBrowser>>>>,
    factory: &Arc<dyn HeadlessBrowserFactory>,
) -> Result<Arc<dyn HeadlessBrowser>, FetchError> {
    let mut guard = slot.lock();
    if let Some(ref browser) = *guard {
        tracing::trace!("SharedBrowser: reusing existing browser");
        return Ok(browser.clone());
    }
    tracing::info!(factory = %factory.name(), "SharedBrowser: launching browser");
    let browser = factory.launch()?;
    *guard = Some(browser.clone());
    Ok(browser)
}

/// Evicts the stored browser from `slot` **only if** it is the same handle as
/// `offender`. Returns the old handle when it evicted.
///
/// This guards against the concurrency race where task A crashes on browser 1,
/// relaunches browser 2, and then task B (still holding its dead clone of
/// browser 1) calls eviction — without the `ptr_eq` check B would evict A's
/// freshly-launched browser 2. Comparing the handle identity scopes eviction
/// to exactly the browser that died.
fn evict_if_matching(
    slot: &Arc<Mutex<Option<Arc<dyn HeadlessBrowser>>>>,
    offender: &Arc<dyn HeadlessBrowser>,
) -> bool {
    let mut guard = slot.lock();
    if guard
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, offender))
    {
        guard.take();
        true
    } else {
        false
    }
}
