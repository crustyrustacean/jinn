// TC4: A struct absent from a facade cannot be reached.
//
// `SessionView` exposes `session` (write). It does NOT expose `frontend` or
// `provider`. Reaching `view.frontend` must be a compile error (E0609).
//
// (The provider capsule was dissolved in the provider-selection window; the
// invariant is re-anchored on the session facade.)

use jinn_domain::common::app_state::AppState;
use jinn_domain::common::state::State;
use jinn_domain::common::tcaps::mint;

fn main() {
    let state = State::new(AppState::default());
    let cap = mint::mint_session_cap();
    state.with_session(|view| {
        // `frontend` is not a field on SessionView — must be E0609.
        let _ = &view.frontend;
    });
}
