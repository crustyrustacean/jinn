//! Chat log - renders the full conversation history.
//!
//! A display-only component showing all messages exchanged in the active session.
//! Each entry type has a distinct visual style (user bold with `>`, system dark gray,
//! actor yellow, assistant cyan). Supports scrolling, selection highlighting,
//! and pinned entry indicators.
//!
//! The entry-to-lines pipeline itself lives in the `jinn-chat-log-view` slice:
//! it is pure, reading no application state. What remains here is the
//! `ChatLogElement` — the `UiElement` that reads `AppState`, resolves the
//! per-frame inputs, and drives scrolling, selection, and the gutter.

pub mod activate_session;
pub mod history;
#[cfg(test)]
mod history_tests;
mod layout_complete;
pub mod layout_supervisor;
#[cfg(test)]
mod layout_tests;
pub(crate) mod layout_worker;

pub use activate_session::activate_session;
pub use history::ChatLogElement;
pub use history::is_session_measured;
pub use layout_complete::{LayoutApplied, LayoutCompletionActor, LayoutCompletionActorDeps};
pub use layout_supervisor::{LAYOUT_DEADLINE, LayoutSupervisorActor, LayoutSupervisorActorDeps};
pub use layout_worker::{LayoutWorkerActor, LayoutWorkerActorDeps, render_preview};

/// Spawns the chat log layout subsystem: the worker pool, its supervisor, and
/// the actor that stores measurements and ends the session load.
///
/// Their typed subscriptions are installed by these spawn calls before it
/// returns, so a load published after activation cannot race startup.
pub fn install_layout_actors(
    system: &trouper::system::ActorSystem,
    state: jinn_domain::common::state::State,
) {
    LayoutSupervisorActor::spawn(
        system,
        LayoutSupervisorActorDeps {
            state: state.clone(),
            system: system.clone(),
        },
    );
    LayoutCompletionActor::spawn(system, LayoutCompletionActorDeps { state });
}

use jinn_domain::common::AppUiRegistry;

/// Register chat log UI element.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(ChatLogElement::new()));
}
