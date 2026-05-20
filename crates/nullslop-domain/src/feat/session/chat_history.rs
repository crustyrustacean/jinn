// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Chat history wrapper — restricts entry pushing to the session feature module.
//!
//! [`ChatHistory`] wraps a `Vec<ChatEntry>` and provides read access via
//! [`Deref<Target = [ChatEntry]>`](std::ops::Deref). The `push` method is
//! restricted with `pub(in crate::feat::session)` visibility so that only
//! code within the session feature module can add new entries. External code
//! must use the `PushChatEntry` command.

use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};

use crate::protocol::ChatEntry;

/// Wrapper around chat history that restricts entry pushing to the session feature module.
///
/// Provides full read access via `Deref<Target = [ChatEntry]>`. The `push`
/// method is restricted so that only session feature code can add entries.
/// Other mutations (compaction inserts, streaming updates) use `pub(crate)`
/// methods that don't push complete entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChatHistory {
    entries: Vec<ChatEntry>,
}

impl ChatHistory {
    /// Create a new empty chat history.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a chat history from an existing vector of entries.
    pub fn from_vec(entries: Vec<ChatEntry>) -> Self {
        Self { entries }
    }

    /// Push an entry onto the history.
    ///
    /// Restricted to the session feature module — external code must use the
    /// `PushChatEntry` command to add entries.
    pub(in crate::feat::session) fn push(&mut self, entry: ChatEntry) {
        self.entries.push(entry);
    }

    /// Insert an entry at a specific position.
    ///
    /// Used by compaction to place entries at boundary positions.
    /// Shifts all entries at or after the insertion point.
    pub(crate) fn insert(&mut self, index: usize, entry: ChatEntry) {
        self.entries.insert(index, entry);
    }

    /// Replace the entire history with a new set of entries.
    pub(crate) fn replace_all(&mut self, entries: Vec<ChatEntry>) {
        self.entries = entries;
    }
}

impl Deref for ChatHistory {
    type Target = [ChatEntry];

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl DerefMut for ChatHistory {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entries
    }
}

impl From<Vec<ChatEntry>> for ChatHistory {
    fn from(entries: Vec<ChatEntry>) -> Self {
        Self { entries }
    }
}

impl From<ChatHistory> for Vec<ChatEntry> {
    fn from(history: ChatHistory) -> Vec<ChatEntry> {
        history.entries
    }
}
