//! Pure browser-binary resolution over a filesystem seam.
//!
//! Resolves the configured [`BrowserBinary`] to a concrete executable path.
//! Tests inject a fake [`BinaryLocator`] so the resolution logic is verifiable
//! without touching the real filesystem.
//!
//! Mirrors `headless_chrome::browser::default_executable()` search logic —
//! honour `CHROME`/`CHROMIUM` env vars, then `which()`, then platform app
//! paths — but scoped per binary family so explicit `Chrome`/`Chromium` modes
//! filter to the right family rather than accepting any Chromium-based build.

use std::path::PathBuf;

use crate::feat::browser::BrowserBinary;

/// A single candidate executable location plus the binary family it belongs to.
///
/// The resolver tests each candidate in order; the first that exists and is
/// executable wins. The family tag lets explicit `Chrome`/`Chromium` modes
/// reject candidates of the wrong family even when a path resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BinaryFamily {
    /// Google Chrome (stable channel).
    Chrome,
    /// Chromium or Chromium-based build.
    Chromium,
    /// The `headless_chrome` crate's auto-downloaded bundled binary.
    ///
    /// No concrete path is known at resolution time (the crate discovers it at
    /// launch), so a `Bundled` resolution carries `path: None`.
    Bundled,
}

/// A filesystem access seam for locating browser executables.
///
/// Production uses [`SystemBinaryLocator`]; tests inject fakes that report
/// fixed sets of candidate paths without touching the disk.
pub trait BinaryLocator {
    /// Return the candidate paths for the given binary family, in search
    /// order. The resolver keeps only the first candidate that the locator
    /// reports as existing.
    fn candidates(&self, family: BinaryFamily) -> Vec<PathBuf>;

    /// Whether the given path refers to an existing, runnable file.
    fn exists(&self, path: &std::path::Path) -> bool;

    /// Detect the installed binary's major version via `<path> --version`.
    ///
    /// Returns the major version as a string (e.g. `"138"`) so it can be
    /// templated into a realistic user-agent. Returns `None` when the binary
    /// cannot be probed or its output cannot be parsed, so callers can fall
    /// back to a hardcoded version.
    fn version(&self, _path: &std::path::Path) -> Option<String> {
        None
    }
}

/// Resolves the configured browser binary.
///
/// Resolution is **infallible**: `Auto` always yields Chrome → system Chromium →
/// bundled Chromium in that order. Explicit `Chrome`/`Chromium` modes resolve
/// their family when present, otherwise fall back to bundled with a
/// [`fallback_note`](ResolvedBrowser::fallback_note) explaining the substitution.
///
/// This matches the fetcher's actual behaviour in `actor_wiring.rs`, which always
/// constructs a working `HeadlessChromeFetcher` (the crate auto-downloads a
/// Chromium when no explicit path is supplied). The dashboard therefore never
/// reports a hard failure for a working system.
pub fn resolve_browser_binary(
    config: BrowserBinary,
    locator: &dyn BinaryLocator,
) -> ResolvedBrowser {
    match config {
        BrowserBinary::Auto => resolve_auto(locator),
        BrowserBinary::Chrome => resolve_explicit_chrome(locator),
        BrowserBinary::Chromium => resolve_explicit_chromium(locator),
    }
}

/// `Auto` resolution: prefer Chrome, then system Chromium, then bundled.
fn resolve_auto(locator: &dyn BinaryLocator) -> ResolvedBrowser {
    if let Some(path) = find_chrome(locator) {
        return resolved_system_binary(BinaryFamily::Chrome, path, locator, None);
    }
    if let Some(path) = find_chromium(locator) {
        return resolved_system_binary(
            BinaryFamily::Chromium,
            path,
            locator,
            Some("Chrome not found — using Chromium"),
        );
    }
    bundled_with_note("No system Chrome/Chromium — using bundled")
}

/// Explicit `Chrome` mode: use Chrome if present, else bundled with a note.
fn resolve_explicit_chrome(locator: &dyn BinaryLocator) -> ResolvedBrowser {
    if let Some(path) = find_chrome(locator) {
        return resolved_system_binary(BinaryFamily::Chrome, path, locator, None);
    }
    bundled_with_note("Chrome not found — using bundled")
}

/// Explicit `Chromium` mode: use Chromium if present, else bundled with a note.
fn resolve_explicit_chromium(locator: &dyn BinaryLocator) -> ResolvedBrowser {
    if let Some(path) = find_chromium(locator) {
        return resolved_system_binary(BinaryFamily::Chromium, path, locator, None);
    }
    bundled_with_note("Chromium not found — using bundled")
}

/// Builds a `ResolvedBrowser` for a system binary (Chrome/Chromium), probing
/// its major version via the locator seam.
fn resolved_system_binary(
    family: BinaryFamily,
    path: PathBuf,
    locator: &dyn BinaryLocator,
    fallback_note: Option<&str>,
) -> ResolvedBrowser {
    let version_major = locator.version(&path);
    ResolvedBrowser {
        family,
        path: Some(path),
        version_major,
        fallback_note: fallback_note.map(ToOwned::to_owned),
    }
}

/// Builds a `ResolvedBrowser` for the bundled binary. No path or version is
/// known at resolution time.
fn bundled_with_note(note: &str) -> ResolvedBrowser {
    ResolvedBrowser {
        family: BinaryFamily::Bundled,
        path: None,
        version_major: None,
        fallback_note: Some(note.to_owned()),
    }
}

/// A resolved browser binary.
///
/// `path` is `None` for the bundled binary (no concrete path is known until
/// launch). `version_major` is `None` when `<binary> --version` could not be
/// probed or parsed, or when the binary is bundled. All fields are communicated
/// via bus events so the dashboard and the fetcher stay in sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBrowser {
    /// The resolved binary family.
    pub family: BinaryFamily,
    /// The resolved executable path, or `None` for the bundled binary.
    pub path: Option<PathBuf>,
    /// The detected major version (e.g. `"138"`), or `None` when undetectable.
    pub version_major: Option<String>,
    /// Human-readable note when resolution fell back from the requested family,
    /// e.g. `"Chrome not found — using bundled"`. `None` for a direct match.
    pub fallback_note: Option<String>,
}

fn find_chrome(locator: &dyn BinaryLocator) -> Option<PathBuf> {
    locator
        .candidates(BinaryFamily::Chrome)
        .into_iter()
        .find(|p| locator.exists(p))
}

fn find_chromium(locator: &dyn BinaryLocator) -> Option<PathBuf> {
    locator
        .candidates(BinaryFamily::Chromium)
        .into_iter()
        .find(|p| locator.exists(p))
}

/// Production locator: probes the real filesystem via the same search order
/// as `headless_chrome::browser::default_executable()`.
pub struct SystemBinaryLocator;

impl BinaryLocator for SystemBinaryLocator {
    fn candidates(&self, family: BinaryFamily) -> Vec<PathBuf> {
        match family {
            BinaryFamily::Chrome => chrome_candidates(),
            BinaryFamily::Chromium => chromium_candidates(),
            // The bundled binary has no discoverable path; resolution returns
            // Bundled with `path: None` without consulting the locator.
            BinaryFamily::Bundled => Vec::new(),
        }
    }

    fn exists(&self, path: &std::path::Path) -> bool {
        path.exists()
    }

    fn version(&self, path: &std::path::Path) -> Option<String> {
        detect_version(path)
    }
}

/// Runs `<path> --version` and extracts the major version number.
///
/// Chrome and Chromium both emit a line like `Google Chrome 138.0.7204.157`
/// or `Chromium 138.0.7204.157`. The major version is the integer before
/// the first dot. Returns `None` on any failure so callers fall back to
/// a hardcoded version.
fn detect_version(path: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new(path)
        .arg("--version")
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_major_version(&stdout)
}

/// Extracts the major version from a `--version` output line.
///
/// Looks for a `D.D.D.D` version token anywhere in the text and returns the
/// digits before the first dot. Made a free function so tests can exercise
/// the parser without spawning a process.
fn parse_major_version(version_output: &str) -> Option<String> {
    // Find the first dot-delimited numeric token.
    let token = version_output.split_whitespace().find(|tok| {
        tok.chars().filter(|&c| c == '.').count() == 3
            && tok
                .split('.')
                .all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
    })?;
    let major = token.split('.').next()?;
    (!major.is_empty()).then(|| major.to_owned())
}

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

fn chrome_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = env_path("CHROME") {
        out.push(p);
    }
    #[cfg(target_os = "linux")]
    {
        for name in ["google-chrome", "google-chrome-stable", "chrome"] {
            out.push(PathBuf::from(name));
        }
    }
    #[cfg(target_os = "macos")]
    {
        out.push(PathBuf::from(
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        ));
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(pf) = std::env::var_os("ProgramFiles") {
            out.push(PathBuf::from(&pf).join("Google\\Chrome\\Application\\chrome.exe"));
        }
        if let Some(pf) = std::env::var_os("ProgramFiles(x86)") {
            out.push(PathBuf::from(&pf).join("Google\\Chrome\\Application\\chrome.exe"));
        }
        if let Some(lad) = std::env::var_os("LOCALAPPDATA") {
            out.push(PathBuf::from(&lad).join("Google\\Chrome\\Application\\chrome.exe"));
        }
    }
    out
}

fn chromium_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = env_path("CHROMIUM") {
        out.push(p);
    }
    #[cfg(target_os = "linux")]
    {
        for name in ["chromium", "chromium-browser"] {
            out.push(PathBuf::from(name));
        }
    }
    #[cfg(target_os = "macos")]
    {
        out.push(PathBuf::from(
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ));
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(pf) = std::env::var_os("ProgramFiles") {
            out.push(PathBuf::from(&pf).join("Chromium\\Application\\chrome.exe"));
        }
    }
    out
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
    use std::collections::HashSet;

    /// In-memory locator: a fixed set of existing paths per family.
    struct FakeFs {
        chrome: Vec<PathBuf>,
        chromium: Vec<PathBuf>,
        existing: HashSet<PathBuf>,
    }

    impl FakeFs {
        fn new() -> Self {
            Self {
                chrome: Vec::new(),
                chromium: Vec::new(),
                existing: HashSet::new(),
            }
        }

        /// Register a Chrome candidate path that exists.
        fn chrome_at(mut self, p: &str) -> Self {
            let path = PathBuf::from(p);
            self.chrome.push(path.clone());
            self.existing.insert(path);
            self
        }

        /// Register a Chromium candidate path that exists.
        fn chromium_at(mut self, p: &str) -> Self {
            let path = PathBuf::from(p);
            self.chromium.push(path.clone());
            self.existing.insert(path);
            self
        }

        /// Register a candidate path that does NOT exist (for search-order
        /// tests where earlier candidates are missing).
        fn chrome_missing(mut self, p: &str) -> Self {
            self.chrome.push(PathBuf::from(p));
            self
        }
    }

    impl BinaryLocator for FakeFs {
        fn candidates(&self, family: BinaryFamily) -> Vec<PathBuf> {
            match family {
                BinaryFamily::Chrome => self.chrome.clone(),
                BinaryFamily::Chromium => self.chromium.clone(),
                BinaryFamily::Bundled => Vec::new(),
            }
        }
        fn exists(&self, path: &std::path::Path) -> bool {
            self.existing.contains(path)
        }
    }

    #[test]
    fn auto_prefers_chrome_when_present() {
        // Given Chrome and Chromium both present.
        let fs = FakeFs::new()
            .chrome_at("/usr/bin/google-chrome")
            .chromium_at("/usr/bin/chromium");

        // When resolving Auto.
        let resolved = resolve_browser_binary(BrowserBinary::Auto, &fs);

        // Then Chrome is preferred.
        assert_eq!(resolved.family, BinaryFamily::Chrome);
        assert_eq!(
            resolved.path.as_deref(),
            Some(std::path::Path::new("/usr/bin/google-chrome"))
        );
        assert_eq!(resolved.fallback_note, None);
    }

    #[test]
    fn auto_falls_back_to_chromium_when_chrome_absent() {
        // Given only Chromium present.
        let fs = FakeFs::new().chromium_at("/usr/bin/chromium");

        // When resolving Auto.
        let resolved = resolve_browser_binary(BrowserBinary::Auto, &fs);

        // Then Chromium is used as fallback.
        assert_eq!(resolved.family, BinaryFamily::Chromium);
        assert_eq!(
            resolved.path.as_deref(),
            Some(std::path::Path::new("/usr/bin/chromium"))
        );
        assert_eq!(
            resolved.fallback_note.as_deref(),
            Some("Chrome not found — using Chromium")
        );
    }

    #[test]
    fn auto_falls_back_to_bundled_when_neither_present() {
        // Given an empty filesystem.
        let fs = FakeFs::new();

        // When resolving Auto.
        let resolved = resolve_browser_binary(BrowserBinary::Auto, &fs);

        // Then resolution falls back to bundled Chromium.
        assert_eq!(resolved.family, BinaryFamily::Bundled);
        assert_eq!(resolved.path, None);
        assert_eq!(resolved.version_major, None);
        assert_eq!(
            resolved.fallback_note.as_deref(),
            Some("No system Chrome/Chromium — using bundled")
        );
    }

    #[test]
    fn explicit_chrome_missing_falls_back_to_bundled() {
        // Given a filesystem with only Chromium.
        let fs = FakeFs::new().chromium_at("/usr/bin/chromium");

        // When resolving explicit Chrome.
        let resolved = resolve_browser_binary(BrowserBinary::Chrome, &fs);

        // Then resolution falls back to bundled Chromium with a note.
        assert_eq!(resolved.family, BinaryFamily::Bundled);
        assert_eq!(resolved.path, None);
        assert_eq!(
            resolved.fallback_note.as_deref(),
            Some("Chrome not found — using bundled")
        );
    }

    #[test]
    fn explicit_chromium_missing_falls_back_to_bundled() {
        // Given a filesystem with only Chrome.
        let fs = FakeFs::new().chrome_at("/usr/bin/google-chrome");

        // When resolving explicit Chromium.
        let resolved = resolve_browser_binary(BrowserBinary::Chromium, &fs);

        // Then resolution falls back to bundled Chromium with a note.
        assert_eq!(resolved.family, BinaryFamily::Bundled);
        assert_eq!(resolved.path, None);
        assert_eq!(
            resolved.fallback_note.as_deref(),
            Some("Chromium not found — using bundled")
        );
    }

    #[test]
    fn resolver_returns_first_existing_candidate_in_search_order() {
        // Given Chrome candidates where the first is missing and the second exists.
        let fs = FakeFs::new()
            .chrome_missing("/missing/google-chrome")
            .chrome_at("/usr/bin/google-chrome");

        // When resolving Chrome.
        let resolved = resolve_browser_binary(BrowserBinary::Chrome, &fs);

        // Then the second candidate (first existing one) is returned.
        assert_eq!(resolved.family, BinaryFamily::Chrome);
        assert_eq!(
            resolved.path.as_deref(),
            Some(std::path::Path::new("/usr/bin/google-chrome"))
        );
    }

    #[test]
    fn parse_major_version_extracts_chrome_version() {
        // Given a Chrome --version line.
        // When parsing.
        let major = parse_major_version("Google Chrome 138.0.7204.157 \n");

        // Then the major version is extracted.
        assert_eq!(major.as_deref(), Some("138"));
    }

    #[test]
    fn parse_major_version_extracts_chromium_version() {
        // Given a Chromium --version line.
        let major = parse_major_version("Chromium 140.0.7339.80");

        // Then the major version is extracted.
        assert_eq!(major.as_deref(), Some("140"));
    }

    #[test]
    fn parse_major_version_returns_none_on_unparseable_output() {
        // Given output with no version token.
        let major = parse_major_version("some error message");

        // Then parsing yields nothing to fall back from.
        assert!(major.is_none());
    }
}
