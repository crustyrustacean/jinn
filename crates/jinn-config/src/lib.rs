//! The configuration layer — `jinn.toml` as one in-memory, reloadable,
//! read+write document.
//!
//! Every `jinn.toml` value in the program is read through this crate and
//! written through it, so a subsystem's configuration is owned by that
//! subsystem instead of by a kernel-side aggregate struct. A section
//! declares itself by implementing [`Configurable`]; the layer stays
//! generic over the type and names no section.

pub mod config_layer;
pub mod configurable;

pub use config_layer::{
    ConfigDocumentStorage, ConfigError, ConfigLayer, FilesystemConfigStorage, InMemoryConfigStorage,
};
pub use configurable::{ConfigList, ConfigSectionError, Configurable, EntryKey};
