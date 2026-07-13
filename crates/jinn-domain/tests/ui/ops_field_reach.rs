// TC2: The private tuple field of an Ops newtype cannot be reached.
//
// `ProviderOps` wraps `&mut ProviderState` in a PRIVATE tuple field. Reaching
// `.0` from inside a projection closure must be a compile error (E0613).
// Only opted-in trait methods (ModelCacheWrite, ProviderPickerWrite, ...) are
// reachable.

use jinn_domain::common::app_state::AppState;
use jinn_domain::common::state::State;
use jinn_domain::common::tcaps::mint;

fn main() {
    let state = State::new(AppState::default());
    let cap = mint::mint_provider_cap();
    state.with_provider(&cap, |view| {
        // Reach the private tuple field — must be E0613.
        let _leaked = view.provider.0;
    });
}
