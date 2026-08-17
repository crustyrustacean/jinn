//! `jinn plugin sdk` — download the plugin SDK crates for local authoring.
//!
//! The SDK is distributed from GitHub releases (not crates.io). This
//! subcommand fetches the SDK archive for the pinned wire version, unpacks
//! it under `~/.local/share/jinn/plugin-sdks/<version>/`, and prints the
//! `path = "..."` lines to paste into a scaffold's `Cargo.toml`. Offline
//! re-runs are free: an already-downloaded version is reused.

use std::path::{Path, PathBuf};

use error_stack::{Report, ResultExt as _};
use wherror::Error;

/// The SDK download failed.
#[derive(Debug, Error, PartialEq, Eq)]
#[error(debug)]
pub enum PluginSdkError {
    /// The network request failed (offline, DNS, GitHub down).
    Download,
    /// The archive could not be unpacked.
    Unpack,
    /// The destination could not be created.
    CreateDir,
    /// A required file is missing from the unpacked SDK.
    MissingCrate,
}

/// The GitHub repo releases are served from.
const RELEASE_BASE: &str = "https://github.com/jinn-app/jinn/releases/download/plugin-sdk";

/// The default SDK version: tracks the wire protocol version.
pub const DEFAULT_SDK_VERSION: &str = "v1";

#[cfg(test)]
thread_local! {
    static RELEASE_BASE_OVERRIDE: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Fetches bytes from a URL — the test seam.
///
/// Production uses reqwest; tests use mockito.
pub trait SdkFetcher: Send + Sync {
    /// Fetches `url` and returns the body.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the status is not 2xx.
    fn fetch(&self, url: &str) -> Result<Vec<u8>, Report<PluginSdkError>>;
}

/// reqwest-backed production fetcher.
///
/// `fetch` blocks on the given runtime handle; the CLI dispatch site owns
/// the runtime and passes it in.
pub struct HttpSdkFetcher {
    handle: tokio::runtime::Handle,
}

impl HttpSdkFetcher {
    /// Creates a fetcher driven by `handle`.
    #[must_use]
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        Self { handle }
    }

    /// Test-only constructor overriding the release base URL.
    #[cfg(test)]
    fn with_base_for_test(base_url: String, handle: tokio::runtime::Handle) -> Self {
        RELEASE_BASE_OVERRIDE.with(|o| o.borrow_mut().replace(base_url));
        Self { handle }
    }
}

impl SdkFetcher for HttpSdkFetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, Report<PluginSdkError>> {
        let url = url.to_owned();
        self.handle.block_on(async move {
            let response = reqwest::get(url.clone())
                .await
                .change_context(PluginSdkError::Download)
                .attach(url.clone())?;
            let status = response.status();
            if !status.is_success() {
                return Err(Report::new(PluginSdkError::Download)
                    .attach(url)
                    .attach(format!("status: {status}")));
            }
            response
                .bytes()
                .await
                .map(|b| b.to_vec())
                .change_context(PluginSdkError::Download)
        })
    }
}

/// Where an SDK version is cached once downloaded.
fn sdk_dir(base: &Path, version: &str) -> PathBuf {
    base.join(version)
}

/// Downloads (or reuses) the SDK for `version`, returning the crate dirs.
///
/// # Errors
///
/// Returns an error if the download, unpack, or dir creation fails.
pub fn acquire(
    version: &str,
    base: &Path,
    fetcher: &dyn SdkFetcher,
) -> Result<SdkPaths, Report<PluginSdkError>> {
    let dir = sdk_dir(base, version);
    let api = dir.join("jinn-plugin-api");
    let sdk = dir.join("jinn-plugin-sdk");
    if api.is_dir() && sdk.is_dir() {
        return Ok(SdkPaths { api, sdk });
    }

    #[cfg(test)]
    let release_base = RELEASE_BASE_OVERRIDE
        .with(|o| o.borrow().clone())
        .unwrap_or_else(|| RELEASE_BASE.to_owned());
    #[cfg(not(test))]
    let release_base = RELEASE_BASE.to_owned();

    let archive = fetcher
        .fetch(&format!("{release_base}/plugin-sdk-{version}.tar.gz"))
        .change_context(PluginSdkError::Download)?;

    std::fs::create_dir_all(&dir)
        .change_context(PluginSdkError::CreateDir)
        .attach(dir.to_string_lossy().to_string())?;

    unpack_tar_gz(&archive, &dir)
        .change_context(PluginSdkError::Unpack)
        .attach(dir.to_string_lossy().to_string())?;

    if !api.is_dir() || !sdk.is_dir() {
        return Err(Report::new(PluginSdkError::MissingCrate)
            .attach(format!("version: {version}"))
            .attach("expected jinn-plugin-api and jinn-plugin-sdk in the archive"));
    }
    Ok(SdkPaths { api, sdk })
}

/// Unpacks an in-memory `.tar.gz` into `dest`.
fn unpack_tar_gz(bytes: &[u8], dest: &Path) -> Result<(), Report<PluginSdkError>> {
    let reader = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(reader);
    archive
        .unpack(dest)
        .change_context(PluginSdkError::Unpack)?;
    Ok(())
}

/// The local paths of the two SDK crates after acquisition.
#[derive(Debug, PartialEq, Eq)]
pub struct SdkPaths {
    /// The wire-types crate dir.
    pub api: PathBuf,
    /// The helper crate dir.
    pub sdk: PathBuf,
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::print_stdout,
        clippy::print_stderr,
        reason = "test assertions"
    )]
    #![expect(clippy::let_underscore_must_use, reason = "none used")]

    use super::*;

    /// A fetcher that returns a canned valid archive built in-memory.
    struct FakeArchive;

    fn build_archive() -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for crate_name in ["jinn-plugin-api", "jinn-plugin-sdk"] {
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, format!("{crate_name}/Cargo.toml"), &b""[..])
                .expect("append");
        }
        let plain = builder.into_inner().expect("finish");
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut gz, &plain).expect("gz");
        gz.finish().expect("finish gz")
    }

    impl SdkFetcher for FakeArchive {
        fn fetch(&self, _url: &str) -> Result<Vec<u8>, Report<PluginSdkError>> {
            Ok(build_archive())
        }
    }

    /// A fetcher that always fails (offline simulation).
    struct Offline;

    impl SdkFetcher for Offline {
        fn fetch(&self, _url: &str) -> Result<Vec<u8>, Report<PluginSdkError>> {
            Err(Report::new(PluginSdkError::Download).attach("offline"))
        }
    }

    // Given an empty cache dir and a working fetcher.
    // When acquiring.
    // Then both crate dirs are unpacked and returned.
    #[test]
    fn acquire_downloads_and_unpacks() {
        // Given an empty temp base.
        let base = std::env::temp_dir().join(format!("jinn-sdk-{}", std::process::id()));

        // When acquiring.
        let paths = acquire("0.1.0", &base, &FakeArchive).expect("acquire");

        // Then both crates exist.
        assert!(paths.api.is_dir());
        assert!(paths.sdk.is_dir());
        assert_eq!(paths.api, base.join("0.1.0/jinn-plugin-api"));

        let _ = std::fs::remove_dir_all(&base);
    }

    // Given an already-downloaded version.
    // When acquiring again.
    // Then the fetcher is not consulted and the same paths return.
    #[test]
    fn acquire_reuses_cached_version() {
        // Given a cached download.
        let base = std::env::temp_dir().join(format!("jinn-sdk-c-{}", std::process::id()));
        acquire("0.1.0", &base, &FakeArchive).expect("first");

        // When acquiring with an always-failing fetcher.
        let paths = acquire("0.1.0", &base, &Offline).expect("second");

        // Then the cached paths still resolve (no download attempted).
        assert!(paths.api.is_dir());

        let _ = std::fs::remove_dir_all(&base);
    }

    // Given a mockito server serving a real archive over HTTP.
    // When acquiring with the production fetcher.
    // Then the archive is downloaded and unpacked.
    #[test]
    fn http_fetcher_downloads_from_url() {
        // Given a sync mockito server and a parked runtime thread.
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/plugin-sdk-0.2.0.tar.gz")
            .with_status(200)
            .with_body(build_archive())
            .create();

        let handle = {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("rt");
                tx.send(rt.handle().clone()).expect("send");
                rt.block_on(std::future::pending::<()>());
            });
            rx.recv().expect("handle")
        };

        let base = std::env::temp_dir().join(format!("jinn-sdk-h-{}", std::process::id()));
        let fetcher = HttpSdkFetcher::with_base_for_test(server.url(), handle);

        // When acquiring with the production fetcher.
        let paths = acquire("0.2.0", &base, &fetcher).expect("acquire");
        mock.assert();

        // Then the archive was downloaded and unpacked.
        assert!(paths.api.is_dir());
        let _ = std::fs::remove_dir_all(&base);
    }

    // Given an offline environment with nothing cached.
    // When acquiring.
    // Then it fails with Download.
    #[test]
    fn acquire_offline_fails_with_download_error() {
        // Given an empty base and no network.
        let base = std::env::temp_dir().join(format!("jinn-sdk-o-{}", std::process::id()));

        // When acquiring.
        let result = acquire("0.1.0", &base, &Offline);

        // Then Download.
        let Err(report) = result else {
            panic!("expected Download");
        };
        assert_eq!(report.current_context(), &PluginSdkError::Download);

        let _ = std::fs::remove_dir_all(&base);
    }
}
