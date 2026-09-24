// TC2: The private tuple field of an Ops newtype cannot be reached.
//
// `SessionOps` wraps `&mut SessionMap` in a PRIVATE tuple field. Reaching `.0`
// from inside a projection closure must be a compile error (E0613). Only
// opted-in accessor methods (`map()`) are reachable.
//
// (The provider capsule was dissolved in the provider-selection window; the
// invariant is re-anchored on the session capsule.)

use jinn_domain::common::app_state::AppState;
use jinn_domain::common::state::State;
use jinn_domain::common::tcaps::mint;

fn main() {
    let state = State::new(AppState::default());
    let cap = mint::mint_session_cap();
    state.with_session(&cap, |view| {
        // Reach the private tuple field — must be E0613.
        let _leaked = view.session.0;
    });
}
