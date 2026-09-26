//! Session-store commands published by frontend and session consumers.

use jinn_core_types::SessionId;
use serde::{Deserialize, Serialize};

/// Request to activate a session, reading it from storage only if absent.
///
/// One command for every activation, because the decision between "read it from
/// disk" and "it is already in memory" belongs to the actor that owns the
/// session map. A caller cannot make it: only it can see what is loaded, and a
/// caller that guessed wrong pays for a redundant read of a session it already
/// had.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Activate a session, loading it from the store if absent.")]
pub struct SessionLoadRequested {
    /// The session to activate.
    pub session_id: SessionId,
    /// The content width the next frame will render at, when the caller knows it.
    ///
    /// Carried rather than derived on arrival: a caller that switches sessions
    /// has already switched by the time the store actor sees this, so the
    /// target's own width is the never-rendered zero, and measuring there
    /// produces counts no frame can use.
    ///
    /// `None` means the caller has no width to offer — Discord publishing this
    /// for a session it has never seen, for instance — and the store actor
    /// derives one from the layout it would render at anyway.
    pub content_width: Option<u16>,
}

impl jinn_slices::BusMessage for SessionLoadRequested {}

/// Request to fork a session at a specific entry ordinal.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Fork a session at an entry ordinal.")]
pub struct SessionForkRequested {
    /// The session to fork from.
    pub source_session_id: SessionId,
    /// Include entries with ordinals through this value in the fork.
    pub at_ordinal: usize,
}

impl jinn_slices::BusMessage for SessionForkRequested {}

/// Load entries for the session picker.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Load session picker entries from the session store.")]
pub struct LoadSessionPickerEntries;

impl jinn_slices::BusMessage for LoadSessionPickerEntries {}

/// Archive a session without running its teardown lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Archive a session without running teardown.")]
pub struct ArchiveSession {
    /// The session to archive.
    pub session_id: SessionId,
}

impl jinn_slices::BusMessage for ArchiveSession {}

/// Archive a session and every descendant in its session tree.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Archive a session and its whole subtree.")]
pub struct ArchiveSessionTree {
    /// The root of the subtree to archive.
    pub root: SessionId,
}

impl jinn_slices::BusMessage for ArchiveSessionTree {}

/// Request to persist a complete session snapshot immediately.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Persist a session snapshot to the store.")]
pub struct PersistSession {
    /// The session to persist.
    pub session_id: SessionId,
}

impl jinn_slices::BusMessage for PersistSession {}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]

    use super::{
        ArchiveSession, ArchiveSessionTree, LoadSessionPickerEntries, PersistSession,
        SessionForkRequested, SessionLoadRequested,
    };
    use jinn_core_types::SessionId;

    #[rstest::rstest]
    #[test]
    fn store_commands_roundtrip_through_json() {
        // Given one of each promoted store command.
        let id = SessionId::new();
        let other_id = SessionId::new();
        let commands = (
            SessionLoadRequested {
                session_id: id.clone(),
                content_width: Some(72),
            },
            SessionForkRequested {
                source_session_id: id.clone(),
                at_ordinal: 7,
            },
            LoadSessionPickerEntries,
            ArchiveSession {
                session_id: id.clone(),
            },
            ArchiveSessionTree { root: id.clone() },
            PersistSession {
                session_id: other_id.clone(),
            },
        );

        // When serializing and deserializing the wire tuple.
        let json = serde_json::to_string(&commands).unwrap();
        let round = serde_json::from_str::<(
            SessionLoadRequested,
            SessionForkRequested,
            LoadSessionPickerEntries,
            ArchiveSession,
            ArchiveSessionTree,
            PersistSession,
        )>(&json)
        .unwrap();

        // Then every command payload survives unchanged.
        assert_eq!(round.0.session_id, id);
        assert_eq!(round.0.content_width, Some(72));
        assert_eq!(round.1.source_session_id, id);
        assert_eq!(round.1.at_ordinal, 7);
        assert_eq!(round.3.session_id, id);
        assert_eq!(round.4.root, id);
        assert_eq!(round.5.session_id, other_id);
    }
}
