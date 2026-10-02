//! The chat-log-view slice — the chat log's display state and actions.
//!
//! Owns two cells: [`chat_log_views_slot`] holds each session's scroll
//! intent and render caches, cursor selection, expand/ignore sets,
//! saved pins position, and ignore-sweep; the audit popup's visibility
//! is a cell of its own because it is a global toggle rather than a
//! per-session one. The log's keys are [`routes`] rows attached at
//! activation — the kernel binds none of them and holds no chat-log
//! intent variant.
//!
//! The view cell's writers are the log's own route actions, reaching it
//! through `ChatSessionState`'s semantic methods (a facade over the
//! cell), and the chat log renderer publishing its per-frame caches
//! through the same methods. There is no actor.

pub mod audit_popup;
pub mod chat_entry_selection;
pub mod chat_log;
pub mod kernel_element;
pub mod minimap_arrow;
pub mod render_regions;
pub mod routes;
pub mod vertical_minimap;

pub use jinn_chat_log_view_msg::ChatLogViewUi;
pub use jinn_chat_log_view_msg::chat_log_scope;
pub use jinn_chat_log_view_msg::chat_log_views_slot;

use jinn_slices::SliceHost;

/// Activates the slice: attaches the log's route rows. No actors, no
/// view.
///
/// Neither cell is minted here: the shared cell catalog
/// (`jinn_cell_catalog::register_all_cells`) registers every slice cell in
/// one place, before any slice activates.
///
/// # Panics
///
/// Panics if the catalog has not run — the route rows and render regions
/// would otherwise act on absent cells and paint nothing.
pub fn activate(host: &mut SliceHost<'_, jinn_slices::RenderFacts>) {
    routes::attach_all(host.key_routes());
    // The chat log's own screen regions: the history itself, the
    // minimap column, and the audit popup.
    render_regions::register(host.slices());
}

/// Register the chat log UI element.
///
/// Called by composition in `jinn-tui`: the slice owns the element, so the
/// kernel's element registry cannot reference it.
pub fn register(registry: &mut jinn_kernel::common::AppUiRegistry) {
    kernel_element::register(registry);
}

#[cfg(test)]
mod activation_tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use jinn_slices::SliceHost;

    /// Registers the cells the catalog owns for this slice.
    ///
    /// The catalog is the production registrar; these tests exercise the
    /// cells themselves, so they seed the same two entries locally rather
    /// than depend on the catalog (which would be a cycle through
    /// `jinn-quake-bar` and `jinn-dashboard`).
    fn seed_chat_log_cells(slices: &jinn_slices::Slices) {
        slices
            .register(
                crate::chat_log_views_slot(),
                jinn_chat_log_view_msg::ChatLogViews::new(),
            )
            .expect("the chat-log-views slot is free in a fresh registry");
        slices
            .register(
                jinn_chat_log_view_msg::audit_popup_slot(),
                jinn_chat_log_view_msg::AuditPopupState::default(),
            )
            .expect("the audit-popup slot is free in a fresh registry");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn activate_registers_the_chat_log_views_cell() {
        // Given a host over a registry holding the catalog's chat-log cells.
        let slices = jinn_slices::Slices::new();
        seed_chat_log_cells(&slices);
        let mut viewport = jinn_slices::view::Viewport::new();
        let overlay_views = jinn_slices::OverlayViews::new();
        let key_routes = jinn_slices::KeyRoutes::new();
        let services = jinn_kernel::Services::new_fake().await;
        let mut host = SliceHost::new(
            &slices,
            &mut viewport,
            &overlay_views,
            &key_routes,
            &services.trouper_system,
        );

        // When activating the slice.
        crate::activate(&mut host);

        // Then the cell resolves and round-trips a per-session write.
        let cell = slices
            .reader::<jinn_chat_log_view_msg::ChatLogViews>(&crate::chat_log_views_slot())
            .expect("activation must register the chat-log-views cell");
        let session_id = jinn_core_types::SessionId::new();
        cell.update(|views| {
            views.entry(session_id.clone()).or_default().scroll_offset = Some(3);
        });
        assert_eq!(
            cell.read().get(&session_id).and_then(|v| v.scroll_offset),
            Some(3),
            "the cell must round-trip a per-session entry"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn per_session_entries_are_isolated() {
        // Given an activated slice with two sessions in the cell.
        let slices = jinn_slices::Slices::new();
        seed_chat_log_cells(&slices);
        let mut viewport = jinn_slices::view::Viewport::new();
        let overlay_views = jinn_slices::OverlayViews::new();
        let key_routes = jinn_slices::KeyRoutes::new();
        let services = jinn_kernel::Services::new_fake().await;
        let mut host = SliceHost::new(
            &slices,
            &mut viewport,
            &overlay_views,
            &key_routes,
            &services.trouper_system,
        );
        crate::activate(&mut host);
        let cell = slices
            .reader::<jinn_chat_log_view_msg::ChatLogViews>(&crate::chat_log_views_slot())
            .expect("activation must register the chat-log-views cell");
        let session_a = jinn_core_types::SessionId::new();
        let session_b = jinn_core_types::SessionId::new();

        // When writing a distinct scroll offset per session.
        cell.update(|views| {
            views.entry(session_a.clone()).or_default().scroll_offset = Some(3);
            views.entry(session_b.clone()).or_default().scroll_offset = Some(7);
        });

        // Then neither session observes the other's offset.
        let views = cell.read();
        assert_eq!(views.get(&session_a).and_then(|v| v.scroll_offset), Some(3));
        assert_eq!(views.get(&session_b).and_then(|v| v.scroll_offset), Some(7));
    }
}
