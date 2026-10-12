//! Tests for the entry-kind → (role, body) extraction that feeds the FTS index.

use jinn_core_types::ChatEntryKind;
use jinn_core_types::session_id::SessionId;

use super::extract::extract_searchable;
use super::model::SearchableRole;

#[rstest::rstest]
#[test]
fn intersession_extracts_as_a_user_role_body() {
    // Given an inter-session kind whose text carries the sender's attribution.
    let kind = ChatEntryKind::Intersession {
        source_session: SessionId::new(),
        source_name: None,
        target_session: SessionId::new(),
        text: "[Message from session abc]: orchestrator says hello".to_owned(),
    };

    // When extracting the searchable view.
    let extracted = extract_searchable(&kind);

    // Then it is indexed under the User role with the full text as the body.
    assert_eq!(
        extracted,
        Some((
            SearchableRole::User,
            "[Message from session abc]: orchestrator says hello".to_owned()
        ))
    );
}

#[rstest::rstest]
#[test]
fn actor_kind_remains_unindexed() {
    // Given an actor kind (hidden agent chatter).
    let kind = ChatEntryKind::Actor {
        source: "worker".to_owned(),
        text: "internal progress note".to_owned(),
    };

    // When extracting the searchable view.
    let extracted = extract_searchable(&kind);

    // Then it yields None — only the intersession arm's membership changed.
    assert_eq!(extracted, None);
}
