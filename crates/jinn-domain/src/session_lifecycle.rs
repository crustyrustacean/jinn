//! Synchronous kernel preparation for the session lifecycle.
//!
//! The lifecycle slice owns the actors that run setup and teardown scripts.
//! What lives here is the other half: the intent handlers that read config,
//! mutate `AppState`, and publish the request. Keeping the split explicit
//! means the slice never has to depend on the kernel to be reachable from it.

pub mod intent;
pub mod validator;
