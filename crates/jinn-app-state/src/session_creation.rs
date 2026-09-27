//! Creating a session from a lifecycle, synchronously.
//!
//! The lifecycle slice owns the actors that run setup and teardown scripts.
//! What lives here is the other half: the operations that read config, mutate
//! [`AppState`](crate::app_state::AppState), and return the messages those
//! actors act on. Both halves belong together on one side of the boundary —
//! the state this mutates and the operations that mutate it — so the
//! operations live beside the state rather than in a caller.

pub mod intent;
pub mod validator;
