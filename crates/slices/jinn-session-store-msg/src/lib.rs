//! Session-store crossing contracts.
//!
//! The search and transcript-window data model lives here because the
//! `SessionStore` seam in `jinn-domain` and the SQLite implementation in
//! `jinn-session-store` both use these types. Neither crate can own them
//! without depending on the other, so they live at the shared low level.

pub mod session_search;

pub use session_search::{
    SearchHit, SearchOutcome, SearchParams, SearchableEntry, SearchableRole, TranscriptEntry,
    TranscriptWindow, entry_ts_key, extract_searchable,
};
