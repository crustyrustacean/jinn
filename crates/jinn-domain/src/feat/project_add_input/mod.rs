//! Project-add input popup - type a directory path to register a new project.
//!
//! Provides a centered text-input popup opened from the project picker (`<c-n>`).
//! The user types a path; the footer shows a live resolved path (green check) or
//! error (red x). On confirm, the path is resolved (`~` expand, relative-to-cwd,
//! canonicalize), validated as an existing dir, and appended to the curated
//! projects list via `UpdatePreferences { AddProject }`.

pub mod intent;
pub mod render;
pub mod state;
