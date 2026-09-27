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

/// Activates the slice: registers the shared token-cache cell, spawns the
/// count + eviction actors on trouper (their `.subscribe` declarations
/// are the readiness point), and returns the cache for composition to
/// hand to the kernel consumers (session actor, prune workers).
///
/// # Panics
///
/// Panics if the slot is already registered — double activation is a
/// wiring bug.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate(
    host: &mut SliceHost<'_, jinn_slices::RenderFacts>,
    state: jinn_kernel::common::state::State,
) {
    let cache = HistoryWorkerChatEntryTokenCache::new();
    let _cell = host
        .register_cell(token_cache_slot(), cache.clone())
        .expect("token-count slot is registered exactly once at wiring");

    let _count_path = count_actor::TokenCountActor::spawn(host.system(), state);
    let _eviction_path =
        eviction_actor::HistoryWorkerChatEntryTokenCacheEvictionActor::spawn(host.system(), cache);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use super::*;
    use jinn_core_types::chat_entry_id::ChatEntryId;
    use jinn_core_types::session_id::SessionId;

    /// A consumer that resolves the cache by slot key gets the same
    /// instance the activation registered: an insert through the cell
    /// handle is visible through an independently-resolved reader (the
    /// session actor's accumulation gate and the prune workers resolve
    /// this way rather than receiving the cache from composition).
    #[rstest::rstest]
    #[tokio::test]
    async fn activate_registers_cell_readable_by_slot_key() {
        // Given an activated slice host.
        let mut services = jinn_kernel::Services::new_fake().await;
        let mut host = jinn_slices::SliceHost::new(
            &services.slices,
            &mut services.viewport,
            &services.overlay_views,
            &services.key_routes,
            &services.trouper_system,
        );

        // When activating and inserting through the registered cell.
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
            .expect("cell registered")
            .read()
            .insert(s.clone(), e.clone(), 33);

        // Then a second, independently-resolved reader observes it.
        let consumer = services
            .slices
            .reader::<HistoryWorkerChatEntryTokenCache>(&token_cache_slot())
            .expect("cell registered");
        assert_eq!(consumer.read().get(&s, &e), Some(33));
    }
}
