// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the Free Software Foundation's version of the GNU Affero
// General Public License as published by the Free Software Foundation.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Citation collector — accumulates consulted web sources across a turn and
//! flushes them as a grouped `Sources` footer when the turn reaches a genuine
//! final assistant answer.
//!
//! This makes provider-independent `web-search` / `web-fetch` tool calls surface
//! a clickable source list, mirroring the footer OpenRouter's server-side
//! `openrouter:web_search` tool produces via `url_citation` annotations — but
//! working on **any** provider (e.g. zai).
//!
//! # How it works
//!
//! The [`CitationCollectorActor`] subscribes to three bus messages:
//!
//! - [`ExecuteWebFetch`] / [`ExecuteWebSearch`] — stash the URL / search query
//!   into a `pending_sources` map keyed by `tool_call_id`.
//! - [`ToolExecutionCompleted`] — on `success`, promote the stashed source into
//!   a per-session [`TurnCitationBuffer`]; on failure, drop it (failed fetches
//!   and searches are not citable).
//! - [`SessionPhaseChanged`] (`Streaming → Idle`) — the flush trigger. Reads the
//!   session's last history entry; if it is an `Assistant` message, flushes the
//!   buffer as a [`CitationsReceived`] event and clears it. Otherwise (error /
//!   cancel mid-turn) the buffer is **retained** so a later successful turn
//!   still surfaces those citations.
//!
//! The actor reuses the entire existing citation pipeline (`CitationsReceived`
//! → session actor → `ChatEntry::annotation` → renderer) and adds no new UI.
//!
//! [`ExecuteWebFetch`]: crate::feat::tools_actor::protocol::command::ExecuteWebFetch
//! [`ExecuteWebSearch`]: crate::feat::tools_actor::protocol::command::ExecuteWebSearch
//! [`ToolExecutionCompleted`]: crate::feat::tools_actor::protocol::event::ToolExecutionCompleted
//! [`SessionPhaseChanged`]: crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged
//! [`CitationsReceived`]: crate::feat::session::protocol::citations_received::CitationsReceived
//! [`CitationCollectorActor`]: citation_collector_actor::CitationCollectorActor

pub mod buffer;
pub mod citation_collector_actor;
