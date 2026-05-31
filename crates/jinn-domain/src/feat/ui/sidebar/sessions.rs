//! Sessions sidebar section - listing, navigation, and session lifecycle actions.
//!
//! This module groups all concerns related to the sessions list in the sidebar:
//! rendering, cursor navigation, session activation, close/archive/teardown
//! handlers, and the lifecycle picker entry point.

pub mod activate;
pub mod archive;
pub mod close;
pub mod r#continue;
pub mod navigate;
pub mod preview;
pub mod reconcile;
pub mod render;

pub mod state;
pub mod teardown;



#[cfg(test)]
mod preview_tests;

use std::time::Duration;

// ---------------------------------------------------------------------------
// Re-exports - preserve the public API for external consumers.
// ---------------------------------------------------------------------------

pub use activate::handle_session_activate;
pub use archive::handle_session_archive;
pub use close::{
    SessionCloseError, handle_session_close, handle_session_close_with_lifecycle,
    validate_session_close,
};
pub use r#continue::handle_session_continue;


pub use navigate::{navigate, receive_cursor, scroll_to_cursor};
pub use preview::{
    SessionPreviewCache, render_session_preview, render_session_preview_for_state,
    session_preview_popup_rect, sessions_section_content_height,
};
pub use reconcile::reconcile_after_session_removal;
pub use render::SessionsSection;
pub use state::SessionsSectionState;
pub use state::clear_visual_parents_on_load;
pub(crate) use state::sorted_open_sessions;
pub use state::update_visual_parents_on_removal;
pub use teardown::handle_session_teardown;

// ---------------------------------------------------------------------------
// Constants - shared across submodules.
// ---------------------------------------------------------------------------

/// Active session indicator prefix.
pub(crate) const ACTIVE_PREFIX: &str = "▸ ";
/// Inactive session prefix (two spaces to align with `ACTIVE_PREFIX`).
pub(crate) const INACTIVE_PREFIX: &str = "  ";
/// Maximum number of session entries visible at once.
pub const MAX_VISIBLE_SESSIONS: usize = 15;
/// Minimum time between animation frame advances.
pub(crate) const ANIMATION_INTERVAL: Duration = Duration::from_millis(80);
