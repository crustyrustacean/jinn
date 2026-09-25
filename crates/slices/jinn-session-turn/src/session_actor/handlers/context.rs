//! Context-related handlers - pinning, caching, and persona management.
//!
//! Handles entry pinning (PinChatEntry/UnpinChatEntry), prompt template
//! caching (PromptTemplatesLoaded), persona selection (PersonasLoaded), and
//! persona picker population (LoadPersonaPickerEntries).
//!
//! Relocated from `PromptAssemblyActor` - these concerns are session-related
//! mutations of `AppState`, not part of prompt assembly.

use jinn_core_types::DEFAULT_PERSONA_NAME;
use jinn_domain::PromptTemplatesLoaded;
use jinn_domain::common::actor_deps::BusPublish;
use jinn_domain::feat::context::protocol::command::LoadPersonaPickerEntries;
use jinn_domain::feat::persona::PersonaEntry;
use jinn_session_history_msg::ChatEntryPinChanged;
use jinn_session_history_msg::{PinChatEntry, UnpinChatEntry};

use super::super::SessionPersistenceActor;

/// Compute sorted pinned-entry IDs from a session, matching
/// `AppState::sorted_pinned_ids` but operating on a session directly so the
/// pins handler can run inside a [`SessionPinsView`] without `&AppState`.
fn sorted_pinned_ids_from_session(
    session: &jinn_session_state::ChatSessionState,
) -> Vec<jinn_core_types::ChatEntryId> {
    use jinn_core_types::ChatEntryId;
    use jinn_domain::common::app_state::pin_sort_key;
    let mut pinned = session.pinned_entries();
    pinned.sort_by_key(|entry| pin_sort_key(entry.pin_position));
    pinned
        .iter()
        .map(|e| e.id.clone())
        .collect::<Vec<ChatEntryId>>()
}

impl SessionPersistenceActor {
    /// PinChatEntry: pin entry in session.
    ///
    /// Pinning is an interaction: the session is marked interacted so the
    /// pin (and the entry it anchors) reaches the store — a pin on a
    /// brand-new, never-sent-to session would otherwise be silently dropped
    /// by the `is_persistable` guard.
    pub(in crate::session_actor) async fn handle_pin_chat_entry(&self, payload: &PinChatEntry) {
        self.state.with_session(|view| {
            let session = view.session.map().get_or_create(&payload.session_id);
            session.pin_entry(&payload.entry_id, payload.position);
            session.mark_interacted();
        });
        self.publish(ChatEntryPinChanged {
            session_id: payload.session_id.clone(),
        })
        .await;
    }

    /// UnpinChatEntry: unpin entry in session.
    ///
    /// Like pinning, unpinning is an interaction and marks the session
    /// interacted so the removal persists.
    pub(in crate::session_actor) async fn handle_unpin_chat_entry(&self, payload: &UnpinChatEntry) {
        {
            self.state
                .with_session_pins(|view| {
                    let is_active = view.session.map().active_session_id() == &payload.session_id;
                    let old_index = if is_active {
                        view.frontend.with_sections(
                            |s| {
                                s.pins.selection_index(&sorted_pinned_ids_from_session(
                                    view.session.map().active_session(),
                                ))
                            },
                            || 0,
                        )
                    } else {
                        0
                    };

                    let session = view.session.map().get_or_create(&payload.session_id);
                    session.unpin_entry(&payload.entry_id);
                    session.mark_interacted();

                    if is_active {
                        let new_sorted =
                            sorted_pinned_ids_from_session(view.session.map().active_session());
                        view.frontend
                            .update_sections(|s| s.pins.clamp_to_nearest(&new_sorted, old_index));
                    }
                });
        }
        self.publish(ChatEntryPinChanged {
            session_id: payload.session_id.clone(),
        })
        .await;
    }

    /// No-op receiver for [`PromptTemplatesLoaded`].
    #[expect(
        clippy::unused_self,
        reason = "trait contract requires #[allow(clippy::unused_self)]self method"
    )]
    ///
    /// The session-init slice's discovery worker writes each session's
    /// discovered prompt set directly into that session's ephemeral state
    /// before emitting the event,
    /// so there is no global mirror to update. The handler exists only to keep
    /// the event dispatch arm explicit (and to make future per-session-side
    /// reactions easy to add).
    pub(in crate::session_actor) fn on_prompt_templates_loaded(
        &self,
        event: &PromptTemplatesLoaded,
    ) {
        tracing::trace!(
            session_id = %event.session_id,
            count = event.templates.len(),
            "prompt templates loaded for session (no global mirror)",
        );
    }

    /// Stores loaded personas in state and selects the active persona.
    ///
    /// Priority:
    /// 1. Honor state.persona_name if set and found in list.
    /// 2. Keep current active_persona if it still exists in the new list.
    /// 3. Fallback to `"coding-assistant"` by name.
    /// 4. If coding-assistant not found, pick first available.
    pub(in crate::session_actor) fn on_personas_loaded(
        &self,
        payload: &jinn_domain::feat::context::protocol::event::PersonasLoaded,
    ) {
        if payload.error.is_some() {
            tracing::warn!(
                error = ?payload.error,
                "persona scan reported an error"
            );
            return;
        }

        // Read frontend.app_state.persona_name (if set) before writing.
        let seeded_persona_name = self.state.read().frontend.app_state.persona_name.clone();

        // The persona catalog lives in the persona slice's cell now; the
        // actor only carries authority to write the selection it was
        // seeded with.
        let Some(cell) = self
            .services
            .slices
            .reader::<jinn_persona_msg::Personas>(&jinn_persona_msg::personas_slot())
        else {
            tracing::warn!("persona slice not activated; dropping PersonasLoaded catalog");
            return;
        };
        cell.update(|personas| {
            personas.seeded_replace(
                payload.personas.clone(),
                seeded_persona_name.as_deref(),
                DEFAULT_PERSONA_NAME,
            );
        });
    }

    /// Loads persona picker entries into `AppState`.
    pub(in crate::session_actor) fn handle_load_persona_picker_entries(
        &self,
        _payload: &LoadPersonaPickerEntries,
    ) {
        let state = self.state.read();
        let selection = self
            .services
            .slices
            .reader::<jinn_persona_msg::Personas>(&jinn_persona_msg::personas_slot())
            .map(|cell| cell.read().clone());
        drop(state);
        let (_active_name, mut entries): (Option<String>, Vec<PersonaEntry>) = match selection {
            Some(selection) => {
                let active_name = selection.active.clone();
                let theme = self.state.read().frontend.theme.clone();
                let entries = selection
                    .entries
                    .iter()
                    .map(|p| PersonaEntry {
                        name: p.name.clone(),
                        description: p.description.clone(),
                        is_active: active_name.as_ref() == Some(&p.name),
                        theme: theme.clone(),
                    })
                    .collect();
                (active_name, entries)
            }
            None => (None, Vec::new()),
        };

        entries.sort_by_key(|e| e.name.to_lowercase());

        self.state
            .with_persona_picker(|picker| {
                picker.set_items(entries);
            });
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::super::super::helpers::test_actor_with_store_recording;
    use super::*;
    use jinn_core_types::SessionId;
    use jinn_domain::common::app_state::AppState;
    use jinn_domain::common::services::BusAudit;
    use jinn_domain::common::state::State;
    use jinn_domain::feat::context::protocol::event::PersonasLoaded;
    use jinn_domain::feat::persona::Persona;
    use jinn_domain::feat::ui::picker_states::PickerExt;
    use jinn_domain::protocol::{ChatEntryId, PinPosition};

    fn make_persona(name: &str) -> Persona {
        Persona {
            name: name.to_owned(),
            description: String::new(),
            body: String::new(),
        }
    }

    async fn create_actor() -> (
        super::super::super::SessionPersistenceActor,
        State,
        BusAudit,
    ) {
        let state = State::new(AppState::default_with_scope_focus());
        let (actor, audit) = super::super::super::helpers::test_actor_recording().await;
        let actor = super::super::super::SessionPersistenceActor {
            state: state.clone(),
            ..actor
        };
        (actor, state, audit)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn on_personas_loaded_selects_coding_assistant_when_none_active() {
        // Given a session actor with no active persona.
        let (actor, _state, _audit) = create_actor().await;
        let personas = vec![
            make_persona("learning-tutor"),
            make_persona("coding-assistant"),
        ];
        let payload = PersonasLoaded {
            personas,
            error: None,
        };

        // When receiving PersonasLoaded.
        actor.on_personas_loaded(&payload);

        // Then coding-assistant is selected by name, not position.
        let selection = actor
            .services
            .slices
            .reader::<jinn_persona_msg::Personas>(&jinn_persona_msg::personas_slot())
            .expect("personas cell seeded");
        let selection_guard = selection.read();
        assert_eq!(selection_guard.active.as_deref(), Some("coding-assistant"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn on_personas_loaded_keeps_existing_active_persona() {
        // Given a session actor with active persona "learning-tutor".
        let (actor, _state, _audit) = create_actor().await;
        actor
            .services
            .slices
            .reader::<jinn_persona_msg::Personas>(&jinn_persona_msg::personas_slot())
            .expect("personas cell seeded")
            .update(|p| p.active = Some("learning-tutor".to_owned()));
        let personas = vec![
            make_persona("coding-assistant"),
            make_persona("learning-tutor"),
        ];
        let payload = PersonasLoaded {
            personas,
            error: None,
        };

        // When receiving PersonasLoaded.
        actor.on_personas_loaded(&payload);

        // Then learning-tutor is kept (still exists in list).
        let selection = actor
            .services
            .slices
            .reader::<jinn_persona_msg::Personas>(&jinn_persona_msg::personas_slot())
            .expect("personas cell seeded");
        assert_eq!(selection.read().active.as_deref(), Some("learning-tutor"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn on_personas_loaded_falls_back_when_active_missing() {
        // Given a session actor where active persona "foo" was deleted from disk.
        let (actor, _state, _audit) = create_actor().await;
        actor
            .services
            .slices
            .reader::<jinn_persona_msg::Personas>(&jinn_persona_msg::personas_slot())
            .expect("personas cell seeded")
            .update(|p| p.active = Some("foo".to_owned()));
        let personas = vec![make_persona("coding-assistant")];
        let payload = PersonasLoaded {
            personas,
            error: None,
        };

        // When receiving PersonasLoaded.
        actor.on_personas_loaded(&payload);

        // Then falls back to coding-assistant.
        let selection = actor
            .services
            .slices
            .reader::<jinn_persona_msg::Personas>(&jinn_persona_msg::personas_slot())
            .expect("personas cell seeded");
        let selection_guard = selection.read();
        assert_eq!(selection_guard.active.as_deref(), Some("coding-assistant"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn on_personas_loaded_uses_first_when_coding_assistant_missing() {
        // Given a session actor with no coding-assistant in the scanned list.
        let (actor, _state, _audit) = create_actor().await;
        let personas = vec![make_persona("learning-tutor")];
        let payload = PersonasLoaded {
            personas,
            error: None,
        };

        // When receiving PersonasLoaded.
        actor.on_personas_loaded(&payload);

        // Then first available is selected.
        let selection = actor
            .services
            .slices
            .reader::<jinn_persona_msg::Personas>(&jinn_persona_msg::personas_slot())
            .expect("personas cell seeded");
        assert_eq!(selection.read().active.as_deref(), Some("learning-tutor"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn on_personas_loaded_clears_active_when_list_empty() {
        // Given a session actor with some active persona.
        let (actor, _state, _audit) = create_actor().await;
        actor
            .services
            .slices
            .reader::<jinn_persona_msg::Personas>(&jinn_persona_msg::personas_slot())
            .expect("personas cell seeded")
            .update(|p| p.active = Some("foo".to_owned()));
        let payload = PersonasLoaded {
            personas: vec![],
            error: None,
        };

        // When receiving PersonasLoaded with empty list.
        actor.on_personas_loaded(&payload);

        // Then active_persona is None.
        let selection = actor
            .services
            .slices
            .reader::<jinn_persona_msg::Personas>(&jinn_persona_msg::personas_slot())
            .expect("personas cell seeded");
        let selection_guard = selection.read();
        assert!(selection_guard.active.is_none());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn on_personas_loaded_resolves_seeded_persona_name() {
        // Given a session actor with a persisted persona_name in frontend.app_state
        // (the value the store actor's environment handler seeds from state.toml at startup).
        let (actor, state, _audit) = create_actor().await;
        {
            let mut guard = state.write();
            guard.frontend.app_state.persona_name = Some("general".to_owned());
        }
        let payload = PersonasLoaded {
            personas: vec![make_persona("coding-assistant"), make_persona("general")],
            error: None,
        };

        // When receiving PersonasLoaded.
        actor.on_personas_loaded(&payload);

        // Then the seeded persona_name wins over the coding-assistant default.
        let selection = actor
            .services
            .slices
            .reader::<jinn_persona_msg::Personas>(&jinn_persona_msg::personas_slot())
            .expect("personas cell seeded");
        assert_eq!(selection.read().active.as_deref(), Some("general"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_pin_chat_entry_pins_and_emits() {
        // Given a session with a user entry.
        let (actor, state, audit) = create_actor().await;
        let entry_id = {
            let mut guard = state.write();
            let session = guard.active_session_mut();
            let entry = jinn_core_types::ChatEntry::user("hello");
            let id = entry.id.clone();
            session.push_entry(entry);
            id
        };
        let session_id = state.read().session.active_session_id().clone();

        // When pinning the entry.
        actor
            .handle_pin_chat_entry(&PinChatEntry {
                session_id: session_id.clone(),
                entry_id: entry_id.clone(),
                position: PinPosition::Top,
            })
            .await;

        // Then the entry is pinned.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");
        let entry = session
            .history()
            .iter()
            .find(|e| e.id == entry_id)
            .expect("entry");
        assert!(entry.is_pinned(), "expected entry to be pinned");
        drop(guard);

        // And ChatEntryPinChanged event was emitted.
        assert!(
            audit.contains_name("ChatEntryPinChanged"),
            "expected ChatEntryPinChanged event"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_unpin_chat_entry_unpins_and_emits() {
        // Given a session with a pinned entry.
        let (actor, state, audit) = create_actor().await;
        let entry_id = {
            let mut guard = state.write();
            let session = guard.active_session_mut();
            let mut entry = jinn_core_types::ChatEntry::user("hello");
            entry.pin_position = Some(PinPosition::Top);
            let id = entry.id.clone();
            session.push_entry(entry);
            id
        };
        let session_id = state.read().session.active_session_id().clone();

        // When unpinning the entry.
        actor
            .handle_unpin_chat_entry(&UnpinChatEntry {
                session_id: session_id.clone(),
                entry_id: entry_id.clone(),
            })
            .await;

        // Then the entry is no longer pinned.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");
        let entry = session
            .history()
            .iter()
            .find(|e| e.id == entry_id)
            .expect("entry");
        assert!(!entry.is_pinned(), "expected entry to be unpinned");
        drop(guard);

        // And ChatEntryPinChanged event was emitted.
        assert!(
            audit.contains_name("ChatEntryPinChanged"),
            "expected ChatEntryPinChanged event"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn pinning_entry_persists_previously_unsaved_session() {
        // Given a non-interacted (not-yet-persistable) session with an entry.
        let (actor, store, _audit) = test_actor_with_store_recording(vec![]).await;
        let session_id = actor.state.read().session.active_session_id().clone();
        let entry_id = {
            let mut guard = actor.state.write();
            let session = guard.active_session_mut();
            let entry = jinn_core_types::ChatEntry::user("hello");
            let id = entry.id.clone();
            session.push_entry(entry);
            id
        };
        assert!(
            store.last_saved_session(&session_id).is_none(),
            "test setup: nothing saved yet"
        );

        // When pinning the entry, then the actor's ChatEntryPinChanged
        // reaction (persist) runs - the same save the bus dispatch triggers.
        actor
            .handle_pin_chat_entry(&PinChatEntry {
                session_id: session_id.clone(),
                entry_id,
                position: PinPosition::Top,
            })
            .await;
        actor.save_active_session(&session_id).await;

        // Then the session reaches the store (the pin persists).
        assert!(
            store.last_saved_session(&session_id).is_some(),
            "pinning a chat entry must persist the session"
        );
    }

    /// Pushes `n` pinned entries (all `Top`, so display order = insertion order)
    /// into the active session and returns their IDs in insertion order.
    fn push_pinned_entries(state: &State, n: usize) -> Vec<ChatEntryId> {
        let mut guard = state.write();
        let session = guard.active_session_mut();
        (0..n)
            .map(|i| {
                let mut entry = jinn_core_types::ChatEntry::user(format!("entry {i}"));
                entry.pin_position = Some(PinPosition::Top);
                let id = entry.id.clone();
                session.push_entry(entry);
                id
            })
            .collect()
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn unpin_keeps_cursor_position_when_first_removed() {
        // Given 3 pinned entries [A, B, C] with A selected.
        let (actor, state, _audit) = create_actor().await;
        let ids = push_pinned_entries(&state, 3);
        let session_id = state.read().session.active_session_id().clone();
        state.write().frontend.update_sections(|s| {
            s.pins.select_by_id(ids[0].clone());
        });

        // When unpinning A.
        actor
            .handle_unpin_chat_entry(&UnpinChatEntry {
                session_id,
                entry_id: ids[0].clone(),
            })
            .await;

        // Then the cursor lands on B (now at index 0).
        assert_eq!(
            state
                .read()
                .frontend
                .with_sections(|s| s.pins.selected_id().cloned(), || None),
            Some(ids[1].clone())
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn unpin_keeps_cursor_position_when_middle_removed() {
        // Given 3 pinned entries [A, B, C] with B selected.
        let (actor, state, _audit) = create_actor().await;
        let ids = push_pinned_entries(&state, 3);
        let session_id = state.read().session.active_session_id().clone();
        state.write().frontend.update_sections(|s| {
            s.pins.select_by_id(ids[1].clone());
        });

        // When unpinning B.
        actor
            .handle_unpin_chat_entry(&UnpinChatEntry {
                session_id,
                entry_id: ids[1].clone(),
            })
            .await;

        // Then the cursor lands on C (now at index 1).
        assert_eq!(
            state
                .read()
                .frontend
                .with_sections(|s| s.pins.selected_id().cloned(), || None),
            Some(ids[2].clone())
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn unpin_keeps_cursor_when_a_different_entry_is_removed() {
        // Given 3 pinned entries [A, B, C] with B selected.
        let (actor, state, _audit) = create_actor().await;
        let ids = push_pinned_entries(&state, 3);
        let session_id = state.read().session.active_session_id().clone();
        let selected = ids[1].clone();
        state.write().frontend.update_sections(|s| {
            s.pins.select_by_id(selected.clone());
        });

        // When unpinning A (a different, non-selected entry).
        actor
            .handle_unpin_chat_entry(&UnpinChatEntry {
                session_id,
                entry_id: ids[0].clone(),
            })
            .await;

        // Then the cursor stays on B (its ID is still present).
        assert_eq!(
            state
                .read()
                .frontend
                .with_sections(|s| s.pins.selected_id().cloned(), || None),
            Some(selected)
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn unpin_clamps_cursor_to_new_last_when_last_removed() {
        // Given 3 pinned entries [A, B, C] with C selected.
        let (actor, state, _audit) = create_actor().await;
        let ids = push_pinned_entries(&state, 3);
        let session_id = state.read().session.active_session_id().clone();
        state.write().frontend.update_sections(|s| {
            s.pins.select_by_id(ids[2].clone());
        });

        // When unpinning C.
        actor
            .handle_unpin_chat_entry(&UnpinChatEntry {
                session_id,
                entry_id: ids[2].clone(),
            })
            .await;

        // Then the cursor clamps to B (the new last entry).
        assert_eq!(
            state
                .read()
                .frontend
                .with_sections(|s| s.pins.selected_id().cloned(), || None),
            Some(ids[1].clone())
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn unpin_clears_cursor_when_only_pin_removed() {
        // Given 1 pinned entry [A] with A selected.
        let (actor, state, _audit) = create_actor().await;
        let ids = push_pinned_entries(&state, 1);
        let session_id = state.read().session.active_session_id().clone();
        state.write().frontend.update_sections(|s| {
            s.pins.select_by_id(ids[0].clone());
        });

        // When unpinning A.
        actor
            .handle_unpin_chat_entry(&UnpinChatEntry {
                session_id,
                entry_id: ids[0].clone(),
            })
            .await;

        // Then the cursor is cleared.
        assert!(
            state
                .read()
                .frontend
                .with_sections(|s| s.pins.selected_id().is_none(), || true)
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn unpin_for_non_active_session_leaves_cursor_unchanged() {
        // Given 3 pinned entries [A, B, C] with B selected, and an unrelated session.
        let (actor, state, _audit) = create_actor().await;
        let ids = push_pinned_entries(&state, 3);
        let selected = ids[1].clone();
        state.write().frontend.update_sections(|s| {
            s.pins.select_by_id(selected.clone());
        });

        // When unpinning B under a non-active session id.
        actor
            .handle_unpin_chat_entry(&UnpinChatEntry {
                session_id: SessionId::new(),
                entry_id: ids[1].clone(),
            })
            .await;

        // Then the cursor is unchanged.
        assert_eq!(
            state
                .read()
                .frontend
                .with_sections(|s| s.pins.selected_id().cloned(), || None),
            Some(selected)
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_load_persona_picker_entries_populates_picker() {
        // Given a session actor with personas loaded.
        let (actor, state, _audit) = create_actor().await;
        actor
            .services
            .slices
            .reader::<jinn_persona_msg::Personas>(&jinn_persona_msg::personas_slot())
            .expect("personas cell seeded")
            .update(|p| {
                p.entries = vec![
                    make_persona("coding-assistant"),
                    make_persona("learning-tutor"),
                ];
                p.active = Some("learning-tutor".to_owned());
            });

        // When loading persona picker entries.
        actor.handle_load_persona_picker_entries(&LoadPersonaPickerEntries);

        // Then the picker has entries with correct active state.
        let guard = state.read();
        let items = guard.frontend.persona_picker().items();
        assert_eq!(items.len(), 2, "expected 2 persona entries");
        let active = items
            .iter()
            .find(|e| e.entry().is_active)
            .expect("an active entry");
        assert_eq!(active.entry().name, "learning-tutor");
    }
}
