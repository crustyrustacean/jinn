//! Local embedding models behind the server's vector tools.
//!
//! The overgraph server embeds text on the local CPU via [`fastembed`]:
//! [`graph_upsert_node`](crate::server) accepts an `embed_text` argument and
//! [`graph_vector_search`](crate::server) embeds its query server-side, so an
//! agent never touches a vector.
//!
//! # Shape
//!
//! - [`EmbedderModel`] — the catalog of known models (dimension, metric, name),
//!   parsed from the `--embed-model` CLI arg.
//! - [`Embedder`] — the backend trait; production runs
//!   [`FastembedEmbedder`], tests inject a deterministic fake. [`embed_async`]
//!   hops any backend onto a blocking thread.
//! - [`EmbedError`] — why an embed (or the model download behind it) failed.
//!
//! # Model lifecycle
//!
//! The model loads lazily at the first embed and stays resident until the
//! process exits — there is no unload path. Weights download from HuggingFace
//! on first use into [`AppPaths::embed_models_dir`] (pre-fetchable headlessly
//! via `jinn mcp overgraph --fetch-model`). Every init is bracketed by
//! explicit stderr lines (see [`announce`]); the model's own hf-hub progress
//! bar is disabled so the inspector log stream stays legible. This crate
//! installs no tracing subscriber, so stderr is its only user-visible
//! channel.
//!
//! A failed init is not cached: the next embed retries the download, so a
//! transient network failure heals without a restart.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Instant;

use error_stack::{Report, ResultExt};
use wherror::Error;

use jinn_common::app_paths::AppPaths;

/// Why an embed failed: the model is unavailable (cold cache + failed
/// download) or the backend rejected the text.
///
/// Callers render this as tool-level error content (`<tool> failed: ...`),
/// so the message must name the failed model download by name.
#[derive(Debug, Clone, Error)]
#[error(debug)]
pub struct EmbedError {
    /// What went wrong, in tool-error prose.
    reason: String,
}

impl EmbedError {
    /// Wraps a backend failure with the reason the embed could not run.
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    /// The failure text as it should appear in tool content.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.reason
    }
}

/// The catalog of embedding models the server can run.
///
/// The default is the deliberate choice: small enough to download and run on
/// any CPU, strong enough to beat the older hosted APIs. The dimension is a
/// permanent property of the database (locked at open), so it lives here and
/// nowhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedderModel {
    /// BAAI/bge-small-en-v1.5 — 384-dim, cosine, ~130MB, 512-token context.
    BgeSmallEnV15,
}

impl EmbedderModel {
    /// The catalog default, used when no `--embed-model` is given.
    #[must_use]
    pub const fn default_model() -> Self {
        Self::BgeSmallEnV15
    }

    /// Parses a `--embed-model` value; unknown names yield `None` so the CLI
    /// can render a legible error naming the valid choices.
    #[must_use]
    pub fn from_arg(arg: &str) -> Option<Self> {
        match arg {
            "bge-small-en-v1.5" => Some(Self::BgeSmallEnV15),
            _ => None,
        }
    }

    /// The `--embed-model` spelling.
    #[must_use]
    pub const fn arg_name(&self) -> &'static str {
        match self {
            Self::BgeSmallEnV15 => "bge-small-en-v1.5",
        }
    }

    /// The model's dense-vector dimension; the database locks to this at open.
    #[must_use]
    pub const fn dimension(&self) -> u32 {
        match self {
            Self::BgeSmallEnV15 => 384,
        }
    }

    /// The similarity metric the model was trained for.
    #[must_use]
    pub const fn metric(&self) -> overgraph::types::DenseMetric {
        match self {
            Self::BgeSmallEnV15 => overgraph::types::DenseMetric::Cosine,
        }
    }

    /// The fastembed catalog entry this model maps to.
    #[must_use]
    const fn fastembed_model(self) -> fastembed::EmbeddingModel {
        match self {
            Self::BgeSmallEnV15 => fastembed::EmbeddingModel::BGESmallENV15,
        }
    }

    /// Approximate download size, for the stderr announcement.
    #[must_use]
    const fn approximate_download(self) -> &'static str {
        match self {
            Self::BgeSmallEnV15 => "~130MB",
        }
    }
}

/// The embedding backend: text in, dense vector out.
///
/// Production runs [`FastembedEmbedder`]; tests inject a deterministic fake
/// so the suite never downloads a model. Not async — fastembed is synchronous
/// and potentially slow (cold init downloads ~130MB), so async callers go
/// through [`embed_async`], which hops onto a blocking thread.
pub trait Embedder: Send + Sync {
    /// The dimension of every vector this backend produces; always the
    /// configured model's [`EmbedderModel::dimension`].
    fn dimension(&self) -> u32;

    /// Embeds one text.
    ///
    /// # Errors
    ///
    /// Returns an error when the model is unavailable — most commonly the
    /// cold-cache + failed-download case, which the message must name.
    fn embed(&self, text: &str) -> Result<Vec<f32>, Report<EmbedError>>;
}

/// Embeds one text off the async worker, via [`spawn_blocking`].
///
/// # Errors
///
/// Propagates [`Embedder::embed`]'s errors; a panic in the blocking task
/// (engine-level fault) also lands here as an error.
pub async fn embed_async(
    embedder: &Arc<dyn Embedder>,
    text: &str,
) -> Result<Vec<f32>, Report<EmbedError>> {
    let embedder = Arc::clone(embedder);
    let text = text.to_owned();
    tokio::task::spawn_blocking(move || embedder.embed(&text))
        .await
        .change_context(EmbedError::new("embedding task failed unexpectedly"))?
}

/// The production embedder: fastembed running one local ONNX model on CPU.
///
/// The model initializes lazily inside the first [`Embedder::embed`] call —
/// jinn owns the lifecycle decision that there is no idle-timeout or explicit
/// unload; the weights stay resident until the process exits. Init is
/// serialized under a mutex so concurrent first calls download exactly once,
/// and the stderr download announcement happens inside that critical section.
pub struct FastembedEmbedder {
    model: EmbedderModel,
    cache_dir: PathBuf,
    /// `None` until the first successful init; a failed init stays `None` so
    /// the next embed retries the download. The model sits behind its own
    /// mutex because fastembed's `embed` takes `&mut self`.
    model_cell: Mutex<Option<Arc<Mutex<fastembed::TextEmbedding>>>>,
}

impl FastembedEmbedder {
    /// Builds an embedder for `model`, caching weights under `cache_dir`.
    ///
    /// No I/O happens here; the model loads (and downloads, if the cache is
    /// cold) at the first [`Embedder::embed`].
    #[must_use]
    pub fn new(model: EmbedderModel, cache_dir: PathBuf) -> Self {
        Self {
            model,
            cache_dir,
            model_cell: Mutex::new(None),
        }
    }

    /// Builds an embedder for `model` at the jinn-managed default cache dir.
    #[must_use]
    pub fn with_default_cache(model: EmbedderModel) -> Self {
        Self::new(model, AppPaths::default().embed_models_dir())
    }

    /// The model this embedder runs.
    #[must_use]
    pub const fn model(&self) -> EmbedderModel {
        self.model
    }

    /// Force-initializes the model now: `--fetch-model`'s whole job.
    ///
    /// Idempotent; after the first success this is cheap.
    ///
    /// # Errors
    ///
    /// Propagates the init failure (cold cache + failed download, most
    /// commonly). Failures are not cached — call again to retry.
    pub fn init(&self) -> Result<(), Report<EmbedError>> {
        self.ensure_initialized().map(|_| ())
    }

    /// Loads the model if not yet loaded, announcing the download on stderr.
    fn ensure_initialized(
        &self,
    ) -> Result<Arc<Mutex<fastembed::TextEmbedding>>, Report<EmbedError>> {
        // The lock is held across the (potentially long) init: every embed
        // needs the same model, so waiting callers are the correct behavior.
        let mut slot = self
            .model_cell
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(embedding) = slot.as_ref() {
            return Ok(Arc::clone(embedding));
        }
        let initialized = Arc::new(Mutex::new(self.initialize()?));
        *slot = Some(Arc::clone(&initialized));
        Ok(initialized)
    }

    /// Runs the actual fastembed init, announcing the download on stderr.
    fn initialize(&self) -> Result<fastembed::TextEmbedding, Report<EmbedError>> {
        let options = fastembed::TextInitOptions::new(self.model.fastembed_model())
            .with_cache_dir(self.cache_dir.clone())
            .with_show_download_progress(false);

        announce_download_start(self.model, &self.cache_dir);
        let started = Instant::now();
        match fastembed::TextEmbedding::try_new(options) {
            Ok(embedding) => {
                announce_download_finished(self.model, started.elapsed());
                Ok(embedding)
            }
            Err(err) => {
                announce_download_failed(self.model, &err);
                Err(Report::new(EmbedError::new(format!(
                    "embedding model {} could not be loaded (download failed? \
                     pre-fetch with: jinn mcp overgraph --fetch-model): {err}",
                    self.model.arg_name()
                ))))
            }
        }
    }
}

impl Embedder for FastembedEmbedder {
    fn dimension(&self) -> u32 {
        self.model.dimension()
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, Report<EmbedError>> {
        let embedding = self.ensure_initialized()?;
        let mut model = embedding.lock().unwrap_or_else(PoisonError::into_inner);
        let vectors = model.embed(vec![text.to_owned()], None).map_err(|err| {
            Report::new(EmbedError::new(format!(
                "embedding model {} rejected the text: {err}",
                self.model.arg_name()
            )))
        })?;
        vectors.into_iter().next().ok_or_else(|| {
            Report::new(EmbedError::new(format!(
                "embedding model {} returned no vector",
                self.model.arg_name()
            )))
        })
    }
}

/// A deterministic test embedder: hash-derived vectors, no model.
///
/// The suite injects this so no test ever downloads weights. Vectors are
/// derived from a stable hash of the input: same text, same vector — which is
/// all the search tests need to assert ordering and identity.
#[derive(Clone)]
pub struct FakeEmbedder {
    dimension: u32,
    /// `true` = model available; `false` = cold + unfetchable (simulates the
    /// offline failure path).
    available: bool,
}

impl FakeEmbedder {
    /// An available fake at `dimension` dimensions.
    #[must_use]
    pub fn available(dimension: u32) -> Self {
        Self {
            dimension,
            available: true,
        }
    }

    /// A fake that always fails, simulating the cold-cache offline state.
    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            dimension: 384,
            available: false,
        }
    }

    /// Deterministic pseudo-vector: FNV-1a seeded per lane from the text.
    /// Magnitude carries no meaning; determinism and distinctness do.
    fn hash_vector(&self, text: &str) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.dimension as usize);
        for lane in 0..self.dimension {
            let mut seed: u64 = 0xcbf2_9ce4_8422_2325;
            for byte in text.as_bytes() {
                seed ^= u64::from(*byte);
                seed = seed.wrapping_mul(0x0000_0100_0000_01b3);
            }
            seed ^= u64::from(lane).rotate_left(17);
            seed = seed.wrapping_mul(0x0000_0100_0000_01b3);
            let unit = f32::from((seed >> 48) as u16) / f32::from(u16::MAX);
            out.push(unit * 2.0 - 1.0);
        }
        out
    }
}

impl Embedder for FakeEmbedder {
    fn dimension(&self) -> u32 {
        self.dimension
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, Report<EmbedError>> {
        if !self.available {
            return Err(Report::new(EmbedError::new(
                "embedding model bge-small-en-v1.5 could not be loaded \
                 (download failed? pre-fetch with: jinn mcp overgraph --fetch-model)",
            )));
        }
        Ok(self.hash_vector(text))
    }
}

/// Announces a model download start on stderr.
///
/// jinn pipes the child's stderr into the MCP inspector log, so these lines
/// are the user-visible record of the download; the model's own progress bar
/// is disabled to keep the stream legible.
fn announce_download_start(model: EmbedderModel, cache_dir: &std::path::Path) {
    announce(&format!(
        "overgraph: loading embedding model {} ({}) into {} ...",
        model.arg_name(),
        model.approximate_download(),
        cache_dir.display()
    ));
}

/// Announces a finished model init on stderr (download or cache hit).
fn announce_download_finished(model: EmbedderModel, init_took: std::time::Duration) {
    announce(&format!(
        "overgraph: embedding model {} ready (init took {:.1}s)",
        model.arg_name(),
        init_took.as_secs_f32()
    ));
}

/// Announces a failed model init on stderr.
fn announce_download_failed(model: EmbedderModel, err: &impl std::fmt::Display) {
    announce(&format!(
        "overgraph: embedding model {} download failed: {err}",
        model.arg_name()
    ));
}

/// Writes one line to stderr: this crate's only user-visible channel when run
/// as a jinn-managed child (no tracing subscriber is installed).
///
/// Exposed so sibling modules (the ingest pipeline's progress lines) share
/// the same channel and lint carve-out instead of duplicating both.
#[expect(
    clippy::print_stderr,
    reason = "the MCP child's stderr is piped to jinn's inspector log; no tracing subscriber exists in this process"
)]
pub fn announce(message: &str) {
    eprintln!("{message}");
}

/// Tests for the pure parts of the embedder module.
#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "test code asserts with expect for clear failures"
)]
mod tests {
    use super::*;

    #[rstest::rstest]
    #[case("bge-small-en-v1.5", EmbedderModel::BgeSmallEnV15)]
    fn from_arg_parses_known_names(#[case] arg: &str, #[case] expected: EmbedderModel) {
        // Given a known --embed-model spelling.
        // When parsing it.
        let parsed = EmbedderModel::from_arg(arg);
        // Then it lands on the catalog entry.
        assert_eq!(parsed, Some(expected));
    }

    #[rstest::rstest]
    fn from_arg_rejects_unknown_names() {
        // Given a name nothing in the catalog carries.
        // When parsing it.
        let parsed = EmbedderModel::from_arg("nope");
        // Then it is rejected.
        assert_eq!(parsed, None);
    }

    #[rstest::rstest]
    fn default_model_is_bge_small() {
        // Given no --embed-model argument.
        // When picking the default.
        let model = EmbedderModel::default_model();
        // Then it is bge-small-en-v1.5.
        assert_eq!(model, EmbedderModel::BgeSmallEnV15);
    }

    #[rstest::rstest]
    fn bge_small_dimension_is_384_cosine() {
        // Given the default model.
        let model = EmbedderModel::default_model();

        // Then its dimension and metric match the documented catalog entry.
        assert_eq!(model.dimension(), 384);
        assert_eq!(model.metric(), overgraph::types::DenseMetric::Cosine);
    }

    #[rstest::rstest]
    fn fake_embedder_is_deterministic_per_text() {
        // Given an available fake embedder.
        let fake = FakeEmbedder::available(8);

        // When embedding the same text twice.
        let first = fake.embed("hello world").expect("available fake embeds");
        let second = fake.embed("hello world").expect("available fake embeds");

        // Then both vectors are identical, at the fake's dimension.
        assert_eq!(first, second);
        assert_eq!(first.len(), 8);
    }

    #[rstest::rstest]
    fn fake_embedder_distinct_texts_get_distinct_vectors() {
        // Given an available fake embedder.
        let fake = FakeEmbedder::available(8);

        // When embedding two different texts.
        let a = fake.embed("alpha").expect("embeds");
        let b = fake.embed("beta").expect("embeds");

        // Then the vectors differ.
        assert_ne!(a, b);
    }

    #[rstest::rstest]
    fn unavailable_fake_embedder_errors_naming_the_download() {
        // Given an unavailable fake embedder.
        let fake = FakeEmbedder::unavailable();

        // When embedding.
        let result = fake.embed("anything");

        // Then the error names the failed model download.
        let err = result.expect_err("unavailable fake must fail");
        assert!(
            err.current_context().message().contains("download failed"),
            "error must name the download: {}",
            err.current_context().message()
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn embed_async_runs_through_the_blocking_pool() {
        // Given an available fake embedder behind the trait object.
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::available(4));

        // When embedding through the async surface.
        let vector = embed_async(&embedder, "async text").await.expect("embeds");

        // Then the vector matches the sync path — same backend, off-thread.
        assert_eq!(vector, embedder.embed("async text").expect("sync embed"));
    }
}
