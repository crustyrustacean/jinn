//! Application identity constants.
//!
//! Centralizes the application name and derived file names so they are
//! defined in exactly one place. Lives in this crate because it is the
//! most-downstream dependency shared by both `nullslop-domain` and the
//! binary crate.

/// The application name, used for config/data directory naming.
pub const APP_NAME: &str = "nullslop";
