//! `@path` file picker — async directory listing for the `@` autocomplete popup.
//!
//! The `@` autocomplete trigger opens an inline file popup. Because directory
//! reads can be slow (network mounts, large dirs), reads happen on the actor
//! system via [`DirectoryListerActor`], not synchronously in the IntentHandler.
//!
//! Flow: the IntentHandler emits a `jinn_chat_input_msg::ListDirectory` command
//! on `@` activation (and on each `/` descent). The actor does
//! `tokio::task::spawn_blocking` `read_dir`, collects
//! `jinn_chat_input_msg::FileEntry`s, and writes them to
//! `frontend.file_picker` — guarded by a staleness check so a slow earlier
//! read can't overwrite a newer one.

pub mod directory_lister_actor;

#[cfg(test)]
mod tests;

pub use directory_lister_actor::{DirectoryListerActor, DirectoryListerActorDeps};
