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

//! Per-session accumulation of web citations gathered during a turn.
//!
//! [`TurnCitationBuffer`] is a pure-logic container used by the
//! [`CitationCollectorActor`]. It holds the URLs the model consulted
//! (`web-fetch` pages + the `web-search` query URL) keyed by session, dedups
//! by URL, and flushes them all at once when the turn reaches a genuine final
//! assistant answer.
//!
//! No I/O, no traits, no async — just a thin, testable wrapper around a map.
//!
//! [`CitationCollectorActor`]: super::citation_collector_actor::CitationCollectorActor

use std::collections::HashMap;

use jinn_provider::UrlCitation;

use crate::protocol::SessionId;

/// Per-session accumulation of web citations for the current turn.
///
/// Citations are buffered as tool calls succeed (`record`) and released as a
/// single grouped `Sources` footer when the turn ends (`flush`). URLs are
/// deduplicated per session so a model fetching the same page twice (or a
/// search URL colliding with a fetched URL) produces one entry, not many.
#[derive(Debug, Default)]
pub struct TurnCitationBuffer {
    /// Pending citations per session. An empty/absent vec means nothing is
    /// buffered; a `flush` returns `None` for that session.
    inner: HashMap<SessionId, Vec<UrlCitation>>,
}

impl TurnCitationBuffer {
    /// Create an empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a citation for a session, deduplicating by URL.
    ///
    /// If a citation with the same `url` is already buffered for this session,
    /// the new one is ignored. This keeps a single fetch (or a search URL that
    /// matches a later fetched page) from producing duplicate footer lines.
    pub fn record(&mut self, session_id: &SessionId, citation: UrlCitation) {
        let slot = self.inner.entry(session_id.clone()).or_default();
        if !slot.iter().any(|existing| existing.url == citation.url) {
            slot.push(citation);
        }
    }

    /// Flush all citations for a session, clearing the slot.
    ///
    /// Returns `None` if no citations are buffered for the session, so callers
    /// can skip a spurious `CitationsReceived` publish without an extra check.
    pub fn flush(&mut self, session_id: &SessionId) -> Option<Vec<UrlCitation>> {
        self.inner.remove(session_id).filter(|v| !v.is_empty())
    }

    /// Whether the session has any unflushed citations.
    #[must_use]
    pub fn pending(&self, session_id: &SessionId) -> bool {
        self.inner
            .get(session_id)
            .is_some_and(|slot| !slot.is_empty())
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::map_err_ignore,
        reason = "test code"
    )]

    use super::*;
    use jinn_provider::UrlCitation;

    fn cite(url: &str) -> UrlCitation {
        UrlCitation {
            url: url.to_owned(),
            title: url.to_owned(),
            content: None,
            start_index: None,
            end_index: None,
        }
    }

    fn sid(n: u8) -> SessionId {
        SessionId::from(uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, &[n]).to_string())
    }

    #[test]
    fn record_appends_citation_for_session() {
        // Given an empty buffer.
        let mut buf = TurnCitationBuffer::new();
        let s = sid(1);

        // When recording a citation.
        buf.record(&s, cite("https://example.com/"));

        // Then the session has a pending citation.
        assert!(buf.pending(&s));
    }

    #[test]
    fn record_dedups_citations_with_same_url() {
        // Given a buffer with one citation for a URL.
        let mut buf = TurnCitationBuffer::new();
        let s = sid(1);
        buf.record(&s, cite("https://example.com/"));

        // When recording the same URL again with a different title.
        let mut dup = cite("https://example.com/");
        dup.title = "different title".to_owned();
        buf.record(&s, dup);

        // Then only one citation is buffered.
        let flushed = buf.flush(&s).expect("should flush");
        assert_eq!(flushed.len(), 1);
    }

    #[test]
    fn flush_returns_citations_and_clears_slot() {
        // Given a buffer with one citation.
        let mut buf = TurnCitationBuffer::new();
        let s = sid(1);
        buf.record(&s, cite("https://example.com/"));

        // When flushing.
        let flushed = buf.flush(&s);

        let flushed = flushed.expect("should have one citation");
        assert_eq!(flushed.len(), 1);
        assert_eq!(flushed[0].url, "https://example.com/");
        // And the session no longer has pending citations.
        assert!(!buf.pending(&s));
    }

    #[test]
    fn flush_returns_none_when_nothing_buffered() {
        // Given an empty buffer.
        let mut buf = TurnCitationBuffer::new();
        let s = sid(1);

        // When flushing a session that has never been recorded.
        let flushed = buf.flush(&s);

        // Then nothing is returned (no spurious footer).
        assert!(flushed.is_none());
    }

    #[test]
    fn flush_returns_none_after_already_flushed() {
        // Given a buffer that was just flushed.
        let mut buf = TurnCitationBuffer::new();
        let s = sid(1);
        buf.record(&s, cite("https://example.com/"));
        let _ = buf.flush(&s);

        // When flushing again.
        let flushed = buf.flush(&s);

        // Then nothing is returned (already cleared).
        assert!(flushed.is_none());
    }

    #[test]
    fn flush_isolates_sessions() {
        // Given a buffer with citations for two distinct sessions.
        let mut buf = TurnCitationBuffer::new();
        let a = sid(1);
        let b = sid(2);
        buf.record(&a, cite("https://a.example/"));
        buf.record(&b, cite("https://b.example/"));

        // When flushing session A.
        let flushed_a = buf.flush(&a);

        // Then session A's citations are returned.
        assert_eq!(flushed_a.map(|v| v.len()), Some(1));
        // And session B is unaffected.
        assert!(buf.pending(&b));
        assert!(buf.flush(&b).is_some());
    }
}
