// TC3: A wrong-type cap is rejected by the projection method.
//
// `State::with_session` requires `&SessionCap`. Passing `&FrontendCap` must be
// a compile error (E0308). This prevents a cap holder from reaching the wrong
// domain even if they hold another domain's cap.
//
// (The provider capsule was dissolved in the provider-selection window; the
// invariant is re-anchored on the session capsule, which every other
// projection test also exercises.)

use jinn_domain::common::app_state::AppState;
use jinn_domain::common::state::State;
use jinn_domain::common::tcaps::mint;

fn main() {
    let state = State::new(AppState::default());
    let wrong_cap = mint::mint_frontend_cap();
    state.with_session(&wrong_cap, |_view| {
        // Passing FrontendCap where SessionCap is required — must be E0308.
    });
}
