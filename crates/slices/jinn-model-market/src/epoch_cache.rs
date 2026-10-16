//! The epoch cache: download `benchmark_data.zip`, extract exactly the
//! three CSVs, and decide cache-vs-seed by TTL with an injected clock.
//!
//! Layout of the epoch cache dir: three files, `eci_scores.csv`,
//! `benchmark_metadata.csv`, `model_metadata.csv` — never the zip
//! itself, never anything else (locked decision B). A directory holding
//! all three is *complete*; anything less falls back to the compiled
//! seed rather than serving a partial index.
//!
//! Time never enters here except through the `now_ms` closure: TTL
//! freshness and the download timestamp ride one injectable clock, which
//! is what makes the expiry tests honest. The seed bytes are **never**
//! written to disk as a stand-in for a download — a directory on disk
//! means a real fetch happened.

use std::fs;
use std::io::Read;
use std::path::Path;

/// The upstream archive (`research/openrouter-api-model-selection-data.md`
/// §8 — ~2.4 MB, 91 benchmark CSVs plus the three files this slice
/// extracts).
pub const EPOCH_ZIP_URL: &str = "https://epoch.ai/data/benchmark_data.zip";

/// Cache freshness window: one week, matching the issue's "weekly-ish"
/// TTLs and the contract's `EPOCH_TTL = 7 days`.
pub const EPOCH_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1000;

/// The zip member the ECI scores live in (a directory inside the
/// archive).
pub const ZIP_ECI_SCORES: &str = "epoch_capabilities_index/eci_scores.csv";
/// The zip member the benchmark normalization metadata lives in.
pub const ZIP_BENCHMARK_METADATA: &str = "benchmark_metadata.csv";
/// The zip member the model roster lives in.
pub const ZIP_MODEL_METADATA: &str = "model_metadata.csv";

/// The three extracted files, with their zip-member origins.
pub const EXTRACTED_FILES: [(&str, &str); 3] = [
    ("eci_scores.csv", ZIP_ECI_SCORES),
    ("benchmark_metadata.csv", ZIP_BENCHMARK_METADATA),
    ("model_metadata.csv", ZIP_MODEL_METADATA),
];

/// The stamp file recording when the three CSVs were extracted.
pub const FETCHED_AT_FILE: &str = "fetched_at";

/// Why the epoch cache could not be consulted, downloaded, or
/// extracted.
///
/// Variants name what a user can do about the failure; the actor
/// forwards the `Display` into the popup's degraded-mode line.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub enum EpochCacheError {
    /// The download failed (offline, DNS, non-200).
    #[error("epoch data download failed: {status}")]
    Download {
        /// The HTTP status line or transport error, ready to render.
        status: String,
    },
    /// The response body could not be read as a zip or an extraction
    /// target was missing from the archive.
    #[error("epoch zip unusable: {detail}")]
    Zip {
        /// What the archive layer rejected, ready to render.
        detail: String,
    },
    /// The three CSVs could not be written into the cache dir.
    #[error("writing epoch cache dir {dir}: {detail}")]
    Write {
        /// The cache dir the write targeted.
        dir: String,
        /// The filesystem's complaint.
        detail: String,
    },
}

/// The complete parsed content of one cache read, with its freshness
/// stamp.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedEpoch {
    /// The three CSVs' bytes, in the [`EXTRACTED_FILES`] order.
    pub files: Vec<Vec<u8>>,
    /// When the cache was written (ms since the epoch, from the
    /// injected clock).
    pub fetched_at_ms: u64,
}

/// Reads the cache dir when it is complete and fresh.
///
/// Completeness: all three CSVs present. Freshness: the `fetched_at`
/// stamp within `ttl_ms` of `now_ms()`. Either failing returns `None`
/// (the caller falls back to the seed or re-downloads).
pub fn load_fresh<N>(dir: &Path, ttl_ms: u64, now_ms: N) -> Option<CachedEpoch>
where
    N: Fn() -> u64,
{
    let stamp = fs::read_to_string(dir.join(FETCHED_AT_FILE))
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    if now_ms().saturating_sub(stamp) >= ttl_ms {
        return None;
    }
    let mut files = Vec::with_capacity(EXTRACTED_FILES.len());
    for (name, _) in EXTRACTED_FILES {
        files.push(fs::read(dir.join(name)).ok()?);
    }
    Some(CachedEpoch {
        files,
        fetched_at_ms: stamp,
    })
}

/// Reads the cache dir when it is complete, regardless of freshness.
///
/// `<c-r>` uses this after dropping the cache: the drop deletes the dir,
/// so anything still standing is a completed extraction from *this*
/// refresh's download.
pub fn load_any(dir: &Path) -> Option<CachedEpoch> {
    let stamp = fs::read_to_string(dir.join(FETCHED_AT_FILE))
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    let mut files = Vec::with_capacity(EXTRACTED_FILES.len());
    for (name, _) in EXTRACTED_FILES {
        files.push(fs::read(dir.join(name)).ok()?);
    }
    Some(CachedEpoch {
        files,
        fetched_at_ms: stamp,
    })
}

/// Deletes the epoch cache dir if it exists.
///
/// `<c-r>`'s drop half. Missing dir is success (nothing to drop); a
/// failed delete is a real error — refresh must not pretend it
/// invalidated what it could not remove.
///
/// # Errors
///
/// Returns an error when the directory exists but cannot be removed.
pub fn drop_cache(dir: &Path) -> Result<(), error_stack::Report<EpochCacheError>> {
    if !dir.exists() {
        return Ok(());
    }
    fs::remove_dir_all(dir).map_err(|error| {
        error_stack::Report::new(EpochCacheError::Write {
            dir: dir.display().to_string(),
            detail: error.to_string(),
        })
        .attach("failed to drop the epoch cache dir")
    })
}

/// Downloads the zip, extracts exactly the three CSVs into `dir`, and
/// stamps the extraction time — all through the injected pieces so tests
/// serve a local mock and a fake clock.
///
/// The zip is downloaded whole into memory (≤ a few MB), never written
/// to disk; only the three extracted files land in the cache dir.
///
/// # Errors
///
/// Returns an error when the download fails, the archive is unreadable
/// or missing a member, or the extraction write fails.
pub fn download_and_extract(
    dir: &Path,
    zip_bytes: &[u8],
    now_ms: u64,
) -> Result<(), error_stack::Report<EpochCacheError>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|error| {
        error_stack::Report::new(EpochCacheError::Zip {
            detail: error.to_string(),
        })
        .attach("failed to open the downloaded benchmark_data.zip")
    })?;

    fs::create_dir_all(dir).map_err(|error| {
        error_stack::Report::new(EpochCacheError::Write {
            dir: dir.display().to_string(),
            detail: error.to_string(),
        })
        .attach("failed to create the epoch cache dir")
    })?;

    for (name, member) in EXTRACTED_FILES {
        let mut entry = archive.by_name(member).map_err(|error| {
            error_stack::Report::new(EpochCacheError::Zip {
                detail: format!("{member}: {error}"),
            })
            .attach("the archive is missing an extraction target")
        })?;
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes).map_err(|error| {
            error_stack::Report::new(EpochCacheError::Zip {
                detail: format!("{member}: {error}"),
            })
            .attach("failed to read the archive member")
        })?;
        write_file(&dir.join(name), &bytes)?;
    }

    write_file(&dir.join(FETCHED_AT_FILE), now_ms.to_string().as_bytes())?;
    Ok(())
}

/// Downloads the archive from `url` over HTTPS.
///
/// Split from [`download_and_extract`] so tests can serve the zip from a
/// local mock without a TLS stack, and so a future ETag/conditional-get
/// has one seam to live in.
///
/// # Errors
///
/// Returns an error on transport failure or a non-2xx status.
pub async fn fetch_zip(
    client: &reqwest::Client,
    url: &str,
) -> Result<Vec<u8>, error_stack::Report<EpochCacheError>> {
    let response = client.get(url).send().await.map_err(|error| {
        error_stack::Report::new(EpochCacheError::Download {
            status: error.to_string(),
        })
        .attach("requesting the epoch benchmark zip")
    })?;
    if !response.status().is_success() {
        return Err(error_stack::Report::new(EpochCacheError::Download {
            status: response.status().to_string(),
        })
        .attach("epoch benchmark zip download returned an error status"));
    }
    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|error| {
            error_stack::Report::new(EpochCacheError::Download {
                status: error.to_string(),
            })
            .attach("reading the epoch benchmark zip body")
        })
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), error_stack::Report<EpochCacheError>> {
    fs::write(path, bytes).map_err(|error| {
        error_stack::Report::new(EpochCacheError::Write {
            dir: path.display().to_string(),
            detail: error.to_string(),
        })
        .attach("failed to write an epoch cache file")
    })
}
