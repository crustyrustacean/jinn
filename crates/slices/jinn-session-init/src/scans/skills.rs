//! Skills scanning for session-init discovery.
//!
//! The merged scan lives in `jinn_skills` — one implementation shared with
//! the skill picker's reload path — re-exported here so `scans::` stays the
//! discovery vocabulary the worker reads. The suite for the merged scan
//! (and for the single-dir scan it wraps) runs where the code lives, in
//! `jinn_skills::scan`; this module keeps only the discovery-specific
//! ancestor-walk tests' entry points.
//!
//! The single-dir `scan_skills` is likewise `jinn_skills`'s; it was never
//! separately configurable here.

pub use jinn_skills::{scan_skills, scan_skills_merged};
