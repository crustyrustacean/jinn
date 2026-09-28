//! The attendant slice — sessions that watch a parent and re-run on its behalf.
//!
//! An attendant is a peer of its parent: empty history, the parent's
//! environment, and run parameters. This slice owns the trigger — when a
//! parent's turn completes successfully, every loaded attendant with
//! `ParentCompleted` pointing at it re-runs — and the seed/reset/re-run
//! behaviors that shape each run.

pub mod activation;
pub mod rerun;
pub mod trigger_actor;

#[cfg(test)]
mod activation_tests;

#[cfg(test)]
mod rerun_tests;

#[cfg(test)]
mod trigger_actor_tests;

use jinn_kernel::common::state::State;
use jinn_slices::SliceHost;

/// Activates the attendant slice: spawns the trigger actor.
///
/// The actor's `TurnCompleted` subscription is the readiness point — after
/// this call resolves, a published completion cannot be missed.
pub fn activate(host: &mut SliceHost<'_, jinn_slices::RenderFacts>, state: State, services: jinn_kernel::Services) {
    trigger_actor::AttendantTriggerActor::spawn(
        host.system(),
        trigger_actor::AttendantTriggerActorDeps { services, state },
    );
}
