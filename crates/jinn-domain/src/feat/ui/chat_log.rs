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

pub mod history;
#[cfg(test)]
mod history_tests;
pub mod layout_complete;
pub mod layout_supervisor;
#[cfg(test)]
mod layout_tests;
pub(crate) mod layout_worker;

pub use history::ChatLogElement;
pub use layout_complete::{LayoutApplied, LayoutCompletionActor, LayoutCompletionActorDeps};
pub use layout_supervisor::{LAYOUT_DEADLINE, LayoutSupervisorActor, LayoutSupervisorActorDeps};
pub use layout_worker::{LayoutWorkerActor, LayoutWorkerActorDeps};

/// Spawns the chat log layout subsystem: the worker pool, its supervisor, and
/// the actor that stores measurements and ends the session load.
///
/// Their typed subscriptions are installed by these spawn calls before it
/// returns, so a load published after activation cannot race startup.
pub fn install_layout_actors(
    system: &trouper::system::ActorSystem,
    state: crate::common::state::State,
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

use crate::common::AppUiRegistry;

/// Register chat log UI element.
pub fn register(registry: &mut AppUiRegistry) {
    registry.register(Box::new(ChatLogElement::new()));
}
