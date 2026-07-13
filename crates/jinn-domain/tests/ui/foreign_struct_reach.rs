// TC4: A struct absent from a facade cannot be reached.
//
// `ProviderView` exposes `session` (read), `provider_frontend`, and `provider`.
// It does NOT expose `context`. Reaching `view.context` must be a compile error
// (E0609).

use jinn_domain::common::app_state::AppState;
use jinn_domain::common::state::State;
use jinn_domain::common::tcaps::mint;

fn main() {
    let state = State::new(AppState::default());
    let cap = mint::mint_provider_cap();
    state.with_provider(&cap, |view| {
        // `context` is not a field on ProviderView — must be E0609.
        let _ = &view.context;
    });
}
