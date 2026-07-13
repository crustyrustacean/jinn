// TC3: A wrong-type cap is rejected by the projection method.
//
// `State::with_provider` requires `&ProviderCap`. Passing `&SessionCap` must be
// a compile error (E0308). This prevents a cap holder from reaching the wrong
// domain even if they hold another domain's cap.

use jinn_domain::common::app_state::AppState;
use jinn_domain::common::state::State;
use jinn_domain::common::tcaps::mint;

fn main() {
    let state = State::new(AppState::default());
    let wrong_cap = mint::mint_session_cap();
    state.with_provider(&wrong_cap, |_view| {
        // Passing SessionCap where ProviderCap is required — must be E0308.
    });
}
