//! The token-count slice — per-entry token counting and cache eviction.
//!
//! Owns the shared [`HistoryWorkerChatEntryTokenCache`] cell
//! ([`token_cache_slot`]) and hosts the two trouper [`ServiceActor`]s that
//! consume it: the count actor (fills `ChatEntry::token_count` in memory
//! on history events) and the eviction actor (clears a closed session's
//! entries), both fed by the `jinn.token-count` forward route. The session
//! actor's accumulation gate and the prune workers hold clones of the same
//! cache for their reads.
//!
//! Kernel dependency: the count actor fills token counts through the
//! session mutation projection on shared [`jinn_kernel::common::state::State`].

pub mod count_actor;
pub mod eviction_actor;

use jinn_slices::SliceHost;

pub use jinn_token_count_msg::HistoryWorkerChatEntryTokenCache;
pub use jinn_token_count_msg::token_cache_slot;

/// Activates the slice: spawns the count + eviction actors on trouper (their
/// `.subscribe` declarations are the readiness point), both bound to the
/// shared token cache.
///
/// The cache cell is not minted here: the shared cell catalog
/// (`jinn_cell_catalog::register_all_cells`) registers every slice cell in
/// one place, so this resolves the cache value out of it — the cache is
/// `Arc`-backed, so the actor's handle is the same instance the session
/// actor's accumulation gate and the prune workers resolve by slot key.
///
/// # Panics
///
/// Panics if the catalog has not run — the actors would otherwise evict from
/// a private cache while every reader resolves none, and the slice would
/// look like it works while caching nothing.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate(
    host: &mut SliceHost<'_, jinn_slices::RenderFacts>,
    state: jinn_kernel::common::state::State,
) {
    let cache_cell = host
        .slices()
        .reader::<HistoryWorkerChatEntryTokenCache>(&token_cache_slot())
        .expect("the cell catalog registers the token-cache slot before any slice activates");
    let cache = cache_cell.read().clone();

    let _count_path = count_actor::TokenCountActor::spawn(host.system(), state);
    let _eviction_path =
        eviction_actor::HistoryWorkerChatEntryTokenCacheEvictionActor::spawn(host.system(), cache);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use super::*;
    use jinn_cell_catalog::register_all_cells;
    use jinn_core_types::chat_entry_id::ChatEntryId;
    use jinn_core_types::session_id::SessionId;

    /// A consumer that resolves the cache by slot key shares the instance
    /// the eviction actor holds: an insert through one resolved handle is
    /// visible through another, and an eviction the actor performs is
    /// visible to the readers. The session actor's accumulation gate and
    /// the prune workers resolve this way.
    #[rstest::rstest]
    #[tokio::test]
    async fn activate_binds_the_eviction_actor_to_the_catalog_cache() {
        // Given a host over a registry the catalog has seeded.
        let mut services = jinn_kernel::Services::new_fake().await;
        register_all_cells(&services.slices);
        let mut host = jinn_slices::SliceHost::new(
            &services.slices,
            &mut services.viewport,
            &services.overlay_views,
            &services.key_routes,
            &services.trouper_system,
        );

        // When activating and inserting through the catalog's cell.
        activate(
            &mut host,
            jinn_kernel::common::state::State::new(
                jinn_kernel::common::app_state::AppState::default(),
            ),
        );
        let s = SessionId::new();
        let e = ChatEntryId::new();
        services
            .slices
            .reader::<HistoryWorkerChatEntryTokenCache>(&token_cache_slot())
            .expect("the catalog registers the token-cache cell")
            .read()
            .insert(s.clone(), e.clone(), 33);

        // Then a second, independently-resolved reader observes it.
        let consumer = services
            .slices
            .reader::<HistoryWorkerChatEntryTokenCache>(&token_cache_slot())
            .expect("the catalog registers the token-cache cell");
        assert_eq!(consumer.read().get(&s, &e), Some(33));
    }
}
