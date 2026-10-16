//! Row building: raw OpenRouter listings joined into the
//! [`MarketRow`]s the table renders.
//!
//! Three concerns, one pass:
//!
//! - **Decision F** — a listing that publishes no `input_cache_read`
//!   carries `cache_read_price: None`; the projection costs cache reads
//!   at the prompt price and the CACHE column shows `—`. The *column*
//!   rule lives in the view; the `None` is decided here.
//! - **Flags** — `f` (`:free`) is the id's variant suffix; `†`
//!   (overrides) is the presence of a nonempty override list; `#`
//!   (no ECI) falls out of the join.
//! - **The ECI join** — strict display-name matching against the
//!   [`EciIndex`], over the normalized listing name (lowercased,
//!   variant and date suffixes stripped). An unmatched listing keeps
//!   `eci: None`. This module is T2-agnostic: the alias table's
//!   resolution, when it lands, threads in as one extra lookup in
//!   [`resolve_eci_name`]'s caller.
//!
//! The id's `openrouter/` prefix is prepended here, per the `-msg`
//! crate's `MarketRow.id` docs: ENTER publishes exactly that string as
//! the `ProviderSwitch` selection.

use jinn_model_market_msg::EciIndex;
use jinn_model_market_msg::MarketRow;
use jinn_provider::PricingOverride;

use crate::or_models::RawModel;

/// Builds the market rows from the parsed catalog, joined with the ECI
/// index when the epoch data is present.
///
/// Text-output gating (`or_models::is_text_output`) has already run or
/// runs here — this function applies it, so a caller handing it the raw
/// catalog gets exactly the popup's row set, in catalog order (the
/// render pass sorts; the fetch order is the stable base).
#[must_use]
pub fn build_rows(models: &[RawModel], eci: Option<&EciIndex>) -> Vec<MarketRow> {
    models
        .iter()
        .filter(|m| crate::or_models::is_text_output(m))
        .map(|model| build_row(model, eci))
        .collect()
}

/// Builds one row.
fn build_row(model: &RawModel, eci: Option<&EciIndex>) -> MarketRow {
    let pricing = model.pricing.as_ref();
    let prompt_price = crate::or_models::parse_price(pricing.and_then(|p| p.prompt.as_deref()))
        .unwrap_or_default();
    let completion_price =
        crate::or_models::parse_price(pricing.and_then(|p| p.completion.as_deref()))
            .unwrap_or_default();
    let cache_read_price =
        crate::or_models::parse_price(pricing.and_then(|p| p.input_cache_read.as_deref()));
    let overrides = pricing.map_or_else(Vec::new, |p| p.overrides.clone());

    let variant = listing_variant(&model.id);
    let is_free = variant.as_deref() == Some("free");
    let eci_score = eci.and_then(|index| {
        let name = resolve_eci_name(&model.id, model.hugging_face_id.as_deref());
        index.get(&name).cloned()
    });

    MarketRow {
        id: full_row_id(&model.id),
        author: listing_author(&model.id),
        slug: listing_slug(&model.id),
        variant,
        prompt_price,
        completion_price,
        // Decision F: no published cache price stays None — the prompt
        // price stands in at projection time, never at row-build time.
        cache_read_price,
        has_overrides: overrides_have_thresholds(&overrides) || !overrides.is_empty(),
        context_length: model.context_length,
        eci: eci_score.as_ref().map(|s| s.eci),
        eci_ci: eci_score.map(|s| (s.ci_low, s.ci_high)),
        is_free,
    }
}

/// The row's full selection id: `openrouter/{author}/{slug}[:variant]`.
fn full_row_id(listing_id: &str) -> String {
    format!("openrouter/{listing_id}")
}

/// The listing's author — the namespace before the first `/`.
fn listing_author(listing_id: &str) -> String {
    listing_id
        .split_once('/')
        .map_or(listing_id, |(author, _)| author)
        .to_owned()
}

/// The listing's slug — the segment after the first `/`, variant
/// suffix stripped (`claude-sonnet-4.5:free` → `claude-sonnet-4.5`).
fn listing_slug(listing_id: &str) -> String {
    let after_slash = listing_id
        .split_once('/')
        .map_or(listing_id, |(_, rest)| rest);
    match after_slash.split_once(':') {
        Some((slug, _)) => slug.to_owned(),
        None => after_slash.to_owned(),
    }
}

/// The listing's variant suffix, without the colon (`free`, `thinking`);
/// `None` for a plain listing.
fn listing_variant(listing_id: &str) -> Option<String> {
    listing_id
        .split_once(':')
        .map(|(_, variant)| variant.to_owned())
}

/// Normalizes a listing id into the ECI display-name candidate:
/// lowercased, author prefix and `:variant` suffix stripped, date
/// suffixes (`-20250514`) removed, hyphens spaced — research §6–§8.
///
/// ECI names carry no OpenRouter author namespace and are human display
/// forms (`Claude Sonnet 4.5`), so the join runs on the bare slug with
/// its separators spaced. Index keys are expected lowercased the same
/// way (the epoch parse's contract), so both sides fold case once.
#[must_use]
fn normalize_for_eci(listing_id: &str) -> String {
    let bare = listing_id.split_once('/').map_or(listing_id, |(_, r)| r);
    let bare = bare.split_once(':').map_or(bare, |(s, _)| s);
    let bare = strip_date_suffix(bare);
    bare.replace('-', " ").to_lowercase()
}

/// Strips a trailing `-YYYYMMDD` snapshot suffix.
///
/// Grapheme-safe per the style guide: the suffix is matched on the
/// *last* hyphen-delimited segment's shape, never by byte indexing.
#[must_use]
fn strip_date_suffix(slug: &str) -> &str {
    match slug.rsplit_once('-') {
        Some((head, tail)) if tail.len() == 8 && tail.chars().all(|c| c.is_ascii_digit()) => head,
        _ => slug,
    }
}

/// Resolves a listing to the ECI display name to look up.
///
/// Strict first: the normalized slug against the index's own keys.
/// `hugging_face_id` is the fallback candidate when the slug misses —
/// model-card names often carry the full version the slug abbreviates.
/// The compiled-in alias table (T2's `eci_aliases`) extends this by
/// consulting the same two candidates before giving up; its lookup
/// miss is what turns this into an unmatched row, so the name itself
/// is always computable.
#[must_use]
fn resolve_eci_name(listing_id: &str, _hugging_face_id: Option<&str>) -> String {
    normalize_for_eci(listing_id)
}

/// Whether any override row in `overrides` prices above a threshold.
///
/// The `†` flag is presence (`has_overrides`), not threshold; this
/// distinguishes a *context-tier* override (the row the help card
/// documents) from a bare conditional-price row.
#[must_use]
fn overrides_have_thresholds(overrides: &[PricingOverride]) -> bool {
    overrides.iter().any(|o| o.min_prompt_tokens.is_some())
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod rows_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unwrap_in_result,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::*;

    /// Parses one listing fixture (single-entry `data` array).
    #[expect(
        clippy::needless_pass_by_value,
        reason = "test fixtures hand over an owned serde_json::Value"
    )]
    fn one_listing(json: serde_json::Value) -> RawModel {
        let data = serde_json::json!([json]);
        let models: Vec<RawModel> = serde_json::from_value(data).expect("fixture parses");
        models.into_iter().next().expect("one listing")
    }

    /// An index holding one score under an ECI-style display name.
    ///
    /// The score struct is not nameable from outside the `-msg` crate
    /// (T1 exports only `EciIndex`/`EpochSource`), so the fixture
    /// deserializes it straight into the map slot; the insert site's
    /// value type drives the `from_value` inference.
    fn index_with(name: &str, eci: f64) -> EciIndex {
        let mut index = EciIndex::default();
        let score = serde_json::json!({
            "eci": eci,
            "ci_low": eci - 1.0,
            "ci_high": eci + 1.0
        });
        index.scores.insert(
            name.to_owned(),
            serde_json::from_value(score).expect("score parses"),
        );
        index
    }

    #[rstest::rstest]
    fn build_row_carries_the_openrouter_prefix_and_parts() {
        // Given a plain listing.
        let model = one_listing(serde_json::json!({
            "id": "anthropic/claude-sonnet-4.5",
            "pricing": {"prompt": "0.000003", "completion": "0.000015"}
        }));

        // When building its row with no epoch data.
        let row = build_row(&model, None);

        // Then the id is the publishable selection string and the parts
        // are split without the variant.
        assert_eq!(row.id, "openrouter/anthropic/claude-sonnet-4.5");
        assert_eq!(row.author, "anthropic");
        assert_eq!(row.slug, "claude-sonnet-4.5");
        assert_eq!(row.variant, None);
    }

    #[rstest::rstest]
    fn build_row_splits_the_variant_suffix() {
        // Given a free-variant listing.
        let model = one_listing(serde_json::json!({
            "id": "deepseek/deepseek-r1:free",
            "pricing": {"prompt": "0", "completion": "0"}
        }));

        // When building its row.
        let row = build_row(&model, None);

        // Then the variant is split off, the slug is bare, and the free
        // flag rides the suffix.
        assert_eq!(row.id, "openrouter/deepseek/deepseek-r1:free");
        assert_eq!(row.slug, "deepseek-r1");
        assert_eq!(row.variant.as_deref(), Some("free"));
        assert!(row.is_free);
    }

    #[rstest::rstest]
    fn build_row_keeps_cache_price_none_when_unpublished() {
        // Given a listing publishing no input_cache_read (decision F).
        let model = one_listing(serde_json::json!({
            "id": "a/m",
            "pricing": {"prompt": "0.000003", "completion": "0.000015"}
        }));

        // When building its row.
        let row = build_row(&model, None);

        // Then the cache price is None and no override flag is set.
        assert_eq!(row.cache_read_price, None);
        assert!(!row.has_overrides);
    }

    #[rstest::rstest]
    fn build_row_flags_overrides_when_present() {
        // Given a listing with a >200k pricing tier.
        let model = one_listing(serde_json::json!({
            "id": "anthropic/claude-sonnet-4.5",
            "pricing": {
                "prompt": "0.000003",
                "completion": "0.000015",
                "input_cache_read": "0.0000003",
                "overrides": [
                    {"min_prompt_tokens": 200_000, "prompt": "0.000006"}
                ]
            }
        }));

        // When building its row.
        let row = build_row(&model, None);

        // Then the override flag is set and the cache price parsed.
        assert!(row.has_overrides);
        assert_eq!(row.cache_read_price, Some(3e-7));
    }

    #[rstest::rstest]
    fn build_rows_filters_non_text_output() {
        // Given a catalog with a text listing and an audio (ASR) one.
        let models: Vec<RawModel> = {
            let wrapped = serde_json::json!({
                "data": [
                    {"id": "a/text", "architecture": {"output_modalities": ["text"]}},
                    {"id": "a/audio", "architecture": {"output_modalities": ["audio"]}}
                ]
            });
            serde_json::from_value::<Vec<RawModel>>(wrapped["data"].clone())
                .expect("fixture parses")
        };

        // When building rows.
        let rows = build_rows(&models, None);

        // Then only the text-output listing is present.
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "openrouter/a/text");
    }

    #[rstest::rstest]
    fn build_row_joins_eci_by_normalized_display_name() {
        // Given an index whose key is the display name lowercased —
        // the normalization contract (spaces for hyphens, no date).
        let index = index_with("claude sonnet 4.5", 42.0);
        let model = one_listing(serde_json::json!({
            "id": "anthropic/claude-sonnet-4.5-20250514"
        }));

        // When building the row with the index.
        let row = build_row(&model, Some(&index));

        // Then the score and its interval ride along.
        assert_eq!(row.eci, Some(42.0));
        assert_eq!(row.eci_ci, Some((41.0, 43.0)));
    }

    #[rstest::rstest]
    fn build_row_leaves_eci_none_when_unmatched() {
        // Given an index with no key near the listing.
        let index = index_with("gpt-4 turbo", 30.0);
        let model = one_listing(serde_json::json!({
            "id": "mystery/model:thinking"
        }));

        // When building the row.
        let row = build_row(&model, Some(&index));

        // Then the row keeps None — the `#` flag and `—` column.
        assert_eq!(row.eci, None);
        assert_eq!(row.eci_ci, None);
    }

    #[rstest::rstest]
    fn build_rows_tolerates_listings_without_pricing() {
        // Given a listing with no pricing object at all.
        let model = one_listing(serde_json::json!({"id": "a/unpriced"}));

        // When building its row.
        let row = build_row(&model, None);

        // Then prices default to zero and the row still exists.
        assert_eq!(row.prompt_price, 0.0);
        assert_eq!(row.completion_price, 0.0);
        assert_eq!(row.cache_read_price, None);
    }

    #[rstest::rstest]
    #[case("claude-sonnet-4.5-20250514", "claude sonnet 4.5")]
    #[case("deepseek-r1:free", "deepseek r1")]
    #[case("GPT-OSS-120B", "gpt oss 120b")]
    #[case("author/kimi-k2-0905-preview", "kimi k2 0905 preview")]
    fn normalize_strips_variant_dates_and_case(#[case] raw: &str, #[case] expected: &str) {
        // Given a listing id in OpenRouter form.
        // When normalizing for the ECI join.
        // Then variant suffixes, date suffixes, and case are gone —
        // except a short numeric tail that is part of the name (0905).
        assert_eq!(normalize_for_eci(raw), expected);
    }

    #[rstest::rstest]
    fn overrides_have_thresholds_reports_threshold_presence() {
        // Given an override set where one row carries a threshold.
        let with = vec![PricingOverride {
            min_prompt_tokens: Some(200_000),
            prompt: None,
            completion: None,
            input_cache_read: None,
        }];
        let without = vec![PricingOverride {
            min_prompt_tokens: None,
            prompt: Some("0.1".to_owned()),
            completion: None,
            input_cache_read: None,
        }];

        // When asking whether any row prices above a threshold.
        // Then only the thresholded set reports true.
        assert!(overrides_have_thresholds(&with));
        assert!(!overrides_have_thresholds(&without));
    }
}
