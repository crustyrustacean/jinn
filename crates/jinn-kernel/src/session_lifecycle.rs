//! Session creation, re-exported from [`jinn_app_state`].
//!
//! The operations live in `jinn-app-state` beside the [`AppState`] they
//! mutate, so that a slice can create a session without depending on the
//! kernel. This module is a path alias only: the definitions are not
//! duplicated here. The lifecycle slice still owns the actors that run the
//! setup and teardown scripts these operations request.

pub mod intent {
    pub use jinn_app_state::session_creation::intent::*;
}

pub mod validator {
    pub use jinn_app_state::session_creation::validator::*;
}
