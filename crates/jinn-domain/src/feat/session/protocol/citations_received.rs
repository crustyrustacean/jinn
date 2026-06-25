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

//! Citations received event - emitted when a stream carries url_citation
//! annotations (e.g. OpenRouter's web search server tool).
//!
//! The session actor subscribes and appends a single display-only
//! [`Annotation`](crate::protocol::ChatEntryKind::Annotation) entry to the
//! session history. Annotations never re-enter LLM context.

use serde::{Deserialize, Serialize};

use crate::protocol::SessionId;

/// Emitted by the LLM actor when a completed stream accumulated one or more
/// `url_citation` annotations.
///
/// Carries the full citation list so the session actor can record a single
/// grouped `Annotation` entry for the turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitationsReceived {
    /// The session the citations belong to.
    pub session_id: SessionId,
    /// The accumulated `url_citation` annotations for the turn.
    pub citations: Vec<jinn_provider::UrlCitation>,
}

impl crate::common::bus::BusMessage for CitationsReceived {}
