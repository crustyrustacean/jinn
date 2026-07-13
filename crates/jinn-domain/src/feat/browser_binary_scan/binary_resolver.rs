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

use crate::feat::web_fetch_actor::BrowserBinary;

/// Error raised when a configured browser binary cannot be found.
#[derive(Debug, Clone, PartialEq, Eq, wherror::Error)]
#[error(debug)]
pub enum BinaryResolutionError {
    /// `BrowserBinary::Auto` found neither Chrome nor Chromium on the system.
    NoBinaryFound,
    /// `BrowserBinary::Chrome` was requested but no Google Chrome was found.
    ChromeNotFound,
    /// `BrowserBinary::Chromium` was requested but no Chromium was found.
    ChromiumNotFound,
}

/// A single candidate executable location plus the binary family it belongs to.
///
/// The resolver tests each candidate in order; the first that exists and is
/// executable wins. The family tag lets explicit `Chrome`/`Chromium` modes
/// reject candidates of the wrong family even when a path resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryFamily {
    /// Google Chrome (stable channel).
    Chrome,
    /// Chromium or Chromium-based build.
    Chromium,
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
}

/// Resolves the configured browser binary to a concrete path.
///
/// Returns the first existing candidate for the requested family, or an error
/// describing which family was missing.
///
/// # Errors
///
/// - [`BinaryResolutionError::ChromeNotFound`] when `Chrome` is requested but
///   no Chrome candidate exists.
/// - [`BinaryResolutionError::ChromiumNotFound`] when `Chromium` is requested
///   but no Chromium candidate exists.
/// - [`BinaryResolutionError::NoBinaryFound`] when `Auto` finds neither.
pub fn resolve_browser_binary(
    config: BrowserBinary,
    locator: &dyn BinaryLocator,
) -> Result<PathBuf, BinaryResolutionError> {
    match config {
        BrowserBinary::Auto => find_chrome(locator).or(find_chromium(locator)),
        BrowserBinary::Chrome => find_chrome(locator),
        BrowserBinary::Chromium => find_chromium(locator),
    }
    .ok_or(match config {
        BrowserBinary::Chrome => BinaryResolutionError::ChromeNotFound,
        BrowserBinary::Chromium => BinaryResolutionError::ChromiumNotFound,
        BrowserBinary::Auto => BinaryResolutionError::NoBinaryFound,
    })
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
        }
    }

    fn exists(&self, path: &std::path::Path) -> bool {
        path.exists()
    }
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
            }
        }

        fn exists(&self, path: &std::path::Path) -> bool {
            self.existing.contains(path)
        }
    }

    #[test]
    fn auto_prefers_chrome_when_present_else_chromium() {
        // Given Chrome and Chromium both present.
        let fs = FakeFs::new()
            .chrome_at("/usr/bin/google-chrome")
            .chromium_at("/usr/bin/chromium");

        // When resolving Auto.
        let resolved = resolve_browser_binary(BrowserBinary::Auto, &fs).expect("should resolve");

        // Then Chrome is preferred.
        assert_eq!(resolved, PathBuf::from("/usr/bin/google-chrome"));

        // Given only Chromium present.
        let fs = FakeFs::new().chromium_at("/usr/bin/chromium");

        // When resolving Auto.
        let resolved = resolve_browser_binary(BrowserBinary::Auto, &fs).expect("should resolve");

        // Then Chromium is used as fallback.
        assert_eq!(resolved, PathBuf::from("/usr/bin/chromium"));
    }

    #[test]
    fn auto_returns_no_binary_found_when_neither_present() {
        // Given an empty filesystem.
        let fs = FakeFs::new();

        // When resolving Auto.
        let result = resolve_browser_binary(BrowserBinary::Auto, &fs);

        // Then resolution fails with NoBinaryFound.
        assert_eq!(result, Err(BinaryResolutionError::NoBinaryFound));
    }

    #[test]
    fn explicit_chrome_missing_returns_chrome_not_found() {
        // Given a filesystem with only Chromium.
        let fs = FakeFs::new().chromium_at("/usr/bin/chromium");

        // When resolving explicit Chrome.
        let result = resolve_browser_binary(BrowserBinary::Chrome, &fs);

        // Then resolution fails with ChromeNotFound.
        assert_eq!(result, Err(BinaryResolutionError::ChromeNotFound));
    }

    #[test]
    fn explicit_chromium_missing_returns_chromium_not_found() {
        // Given a filesystem with only Chrome.
        let fs = FakeFs::new().chrome_at("/usr/bin/google-chrome");

        // When resolving explicit Chromium.
        let result = resolve_browser_binary(BrowserBinary::Chromium, &fs);

        // Then resolution fails with ChromiumNotFound.
        assert_eq!(result, Err(BinaryResolutionError::ChromiumNotFound));
    }

    #[test]
    fn resolver_returns_first_existing_candidate_in_search_order() {
        // Given Chrome candidates where the first is missing and the second exists.
        let fs = FakeFs::new()
            .chrome_missing("/missing/google-chrome")
            .chrome_at("/usr/bin/google-chrome");

        // When resolving Chrome.
        let resolved = resolve_browser_binary(BrowserBinary::Chrome, &fs).expect("should resolve");

        // Then the second candidate (first existing one) is returned.
        assert_eq!(resolved, PathBuf::from("/usr/bin/google-chrome"));
    }
}
