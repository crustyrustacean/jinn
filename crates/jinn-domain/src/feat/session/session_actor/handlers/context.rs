//! Context-related handlers - pinning, caching, and persona management.
//!
//! Handles entry pinning (PinChatEntry/UnpinChatEntry), tool definition caching
//! (ToolsRegistered), prompt template caching (PromptTemplatesLoaded),
//! persona selection (PersonasLoaded), and persona picker population
//! (LoadPersonaPickerEntries).
//!
//! Relocated from `PromptAssemblyActor` - these concerns are session-related
//! mutations of `AppState`, not part of prompt assembly.

use crate::common::actor_deps::BusPublish;
use crate::feat::context::protocol::command::{
    LoadPersonaPickerEntries, PinChatEntry, UnpinChatEntry,
};
use crate::feat::context::protocol::event::ChatEntryPinChanged;
use crate::feat::persona::PersonaEntry;
use crate::feat::provider::protocol::event::PromptTemplatesLoaded;
use crate::feat::tools_actor::protocol::event::ToolsRegistered;
use crate::feat::ui::picker_states::PickerExt;

use super::super::SessionPersistenceActor;

impl SessionPersistenceActor {
    /// PinChatEntry: pin entry in session.
    pub(in crate::feat::session::session_actor) async fn handle_pin_chat_entry(
        &self,
        payload: &PinChatEntry,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.pin_entry(&payload.entry_id, payload.position);
        }
        self.publish(ChatEntryPinChanged {
            session_id: payload.session_id.clone(),
        })
        .await;
    }

    /// UnpinChatEntry: unpin entry in session.
    pub(in crate::feat::session::session_actor) async fn handle_unpin_chat_entry(
        &self,
        payload: &UnpinChatEntry,
    ) {
        {
            let mut state = self.state.write();
            let is_active = state.session.active_session_id() == &payload.session_id;
            let old_index = if is_active {
                state
                    .frontend
                    .pins
                    .selection_index(&state.sorted_pinned_ids())
            } else {
                0
            };

            let session = state.session_mut_or_create(&payload.session_id);
            session.unpin_entry(&payload.entry_id);

            if is_active {
                let new_sorted = state.sorted_pinned_ids();
                state.frontend.pins.clamp_to_nearest(&new_sorted, old_index);
            }
        }
        self.publish(ChatEntryPinChanged {
            session_id: payload.session_id.clone(),
        })
        .await;
    }

    pub(in crate::feat::session::session_actor) fn on_tools_registered(
        &self,
        evt: &ToolsRegistered,
    ) {
        let mut state = self.state.write();

        match &evt.session_id {
            // Global tools (builtins + global plugins) -> shared map.
            None => {
                for def in &evt.definitions {
                    state
                        .context
                        .global_tool_definitions
                        .insert(def.name.clone(), def.clone());
                }
            }
            // Attached plugin tools -> per-session map.
            Some(target_id) => {
                let session_map = state
                    .context
                    .session_tool_definitions
                    .entry(target_id.clone())
                    .or_default();
                for def in &evt.definitions {
                    session_map.insert(def.name.clone(), def.clone());
                }
            }
        }
    }

    /// No-op receiver for [`PromptTemplatesLoaded`].
    #[expect(
        clippy::unused_self,
        reason = "trait contract requires #[allow(clippy::unused_self)]self method"
    )]
    ///
    /// The [`PromptScanActor`] writes each session's discovered prompt set
    /// directly into that session's ephemeral state before emitting the event,
    /// so there is no global mirror to update. The handler exists only to keep
    /// the event dispatch arm explicit (and to make future per-session-side
    /// reactions easy to add).
    pub(in crate::feat::session::session_actor) fn on_prompt_templates_loaded(
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
    pub(in crate::feat::session::session_actor) fn on_personas_loaded(
        &self,
        payload: &crate::feat::context::protocol::event::PersonasLoaded,
    ) {
        if payload.error.is_some() {
            tracing::warn!(
                error = ?payload.error,
                "persona scan reported an error"
            );
            return;
        }
        let mut state = self.state.write();
        state.context.personas.clone_from(&payload.personas);

        let target_name = state
            .frontend
            .app_state
            .persona_name
            .as_deref()
            .filter(|name| payload.personas.iter().any(|p| p.name == *name))
            .or_else(|| {
                state
                    .context
                    .active_persona
                    .as_ref()
                    .filter(|p| payload.personas.iter().any(|sp| sp.name == p.name))
                    .map(|p| p.name.as_str())
            })
            .unwrap_or("coding-assistant");

        let found = payload
            .personas
            .iter()
            .find(|p| p.name == target_name)
            .cloned();

        if let Some(persona) = found {
            state.context.active_persona = Some(persona);
        } else {
            // Edge case: coding-assistant not found either.
            state.context.active_persona = payload.personas.first().cloned();
        }
    }

    /// Loads persona picker entries into `AppState`.
    pub(in crate::feat::session::session_actor) fn handle_load_persona_picker_entries(
        &self,
        _payload: &LoadPersonaPickerEntries,
    ) {
        let state = self.state.read();
        let active_name = state
            .context
            .active_persona
            .as_ref()
            .map(|p| p.name.clone());
        let mut entries: Vec<PersonaEntry> = state
            .context
            .personas
            .iter()
            .map(|p| PersonaEntry {
                name: p.name.clone(),
                description: p.description.clone(),
                is_active: active_name.as_ref() == Some(&p.name),
                theme: state.frontend.theme.clone(),
            })
            .collect();
        drop(state);

        entries.sort_by_key(|e| e.name.to_lowercase());

        let mut state = self.state.write();
        state.frontend.persona_picker_mut().set_items(entries);
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

    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::services::BusAudit;
    use crate::common::state::State;
    use crate::feat::context::protocol::event::PersonasLoaded;
    use crate::feat::persona::Persona;
    use crate::feat::tools_actor::tool_types::ToolDefinition;
    use crate::protocol::{ChatEntryId, PinPosition, SessionId};

    fn make_persona(name: &str) -> Persona {
        Persona {
            name: name.to_owned(),
            description: String::new(),
            body: String::new(),
            file_path: std::path::PathBuf::from(format!("/personas/{name}.md")),
        }
    }

    async fn create_actor() -> (
        super::super::super::SessionPersistenceActor,
        State,
        BusAudit,
    ) {
        let state = State::new(AppState::default());
        let (actor, audit) = super::super::super::helpers::test_actor_recording().await;
        let actor = super::super::super::SessionPersistenceActor {
            state: state.clone(),
            ..actor
        };
        (actor, state, audit)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn on_tools_registered_keeps_regular_tools_in_global_map() {
        // Given a session actor.
        let (actor, state, _audit) = create_actor().await;

        // Build a ToolsRegistered with all builtin tools.
        let all_tools = crate::feat::tools_actor::registry::builtin_tools(
            &crate::feat::preferences_actor::user_preferences::BashConfig::default(),
        );
        let definitions: Vec<_> = all_tools.iter().map(|(def, _)| def.clone()).collect();
        let payload = ToolsRegistered {
            provider: "builtin".to_owned(),
            definitions,
            session_id: None,
        };

        // When processing the event.
        actor.on_tools_registered(&payload);

        // Then regular tools are in the global map.
        let guard = state.read();
        assert!(
            guard.context.global_tool_definitions.contains_key("bash"),
            "bash should be in global tool map"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn on_tools_registered_ignores_attached_tools_for_different_session() {
        // Given a session actor.
        let (actor, state, _audit) = create_actor().await;
        // Build a ToolsRegistered targeting a different session.
        let other_session_id = SessionId::new();
        let payload = ToolsRegistered {
            provider: "plugin:judge".to_owned(),
            definitions: vec![ToolDefinition {
                name: "judgment_passed".to_owned(),
                description: "Pass".to_owned(),
                prompt_snippet: None,
                prompt_guidelines: vec![],
                parameters: serde_json::json!({"type": "object", "properties": {}}),
                server_tool_type: None,
            }],
            session_id: Some(other_session_id.clone()),
        };

        // When processing the event.
        actor.on_tools_registered(&payload);

        // Then the tool is NOT in any global map.
        let guard = state.read();
        assert!(
            !guard
                .context
                .global_tool_definitions
                .contains_key("judgment_passed"),
            "attached tool for different session should not be in global map"
        );
        // And NOT in the target session's map either (it was stored by session_id key).
        // Since the tool WAS stored in session_tool_definitions[other_session_id],
        // it should be there, not in global.
        let session_tools = guard
            .context
            .session_tool_definitions
            .get(&other_session_id);
        // The tool IS stored under the correct session key (that's the new behavior).
        assert!(
            session_tools.is_some_and(|m| m.contains_key("judgment_passed")),
            "attached tool should be stored under its target session key"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn on_tools_registered_stores_attached_tools_for_own_session() {
        // Given a session actor.
        let (actor, state, _audit) = create_actor().await;
        let session_id = state.read().session.active_session_id().clone();

        // Build a ToolsRegistered targeting this session.
        let payload = ToolsRegistered {
            provider: "plugin:judge".to_owned(),
            definitions: vec![ToolDefinition {
                name: "judgment_passed".to_owned(),
                description: "Pass".to_owned(),
                prompt_snippet: None,
                prompt_guidelines: vec![],
                parameters: serde_json::json!({"type": "object", "properties": {}}),
                server_tool_type: None,
            }],
            session_id: Some(session_id.clone()),
        };

        // When processing the event.
        actor.on_tools_registered(&payload);

        // Then the tool IS stored in the session-specific map.
        let guard = state.read();
        let session_tools = guard.context.session_tool_definitions.get(&session_id);
        assert!(
            session_tools.is_some_and(|m| m.contains_key("judgment_passed")),
            "attached tool for own session should be stored in session map"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn on_tools_registered_stores_global_tools_unconditionally() {
        // Given a session actor.
        let (actor, state, _audit) = create_actor().await;
        // Given a session actor.

        // Build a global ToolsRegistered (session_id: None).
        let payload = ToolsRegistered {
            provider: "plugin:helper".to_owned(),
            definitions: vec![ToolDefinition {
                name: "global_helper".to_owned(),
                description: "Help".to_owned(),
                prompt_snippet: None,
                prompt_guidelines: vec![],
                parameters: serde_json::json!({"type": "object", "properties": {}}),
                server_tool_type: None,
            }],
            session_id: None,
        };

        // When processing the event.
        actor.on_tools_registered(&payload);

        // Then the global tool is stored.
        let guard = state.read();
        assert!(
            guard
                .context
                .global_tool_definitions
                .contains_key("global_helper"),
            "global tool should be stored unconditionally"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn on_personas_loaded_selects_coding_assistant_when_none_active() {
        // Given a session actor with no active persona.
        let (actor, state, _audit) = create_actor().await;
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
        let guard = state.read();
        assert_eq!(
            guard
                .context
                .active_persona
                .as_ref()
                .map(|p| p.name.as_str()),
            Some("coding-assistant")
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn on_personas_loaded_keeps_existing_active_persona() {
        // Given a session actor with active persona "learning-tutor".
        let (actor, state, _audit) = create_actor().await;
        {
            let mut guard = state.write();
            guard.context.active_persona = Some(make_persona("learning-tutor"));
        }
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
        let guard = state.read();
        assert_eq!(
            guard
                .context
                .active_persona
                .as_ref()
                .map(|p| p.name.as_str()),
            Some("learning-tutor")
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn on_personas_loaded_falls_back_when_active_missing() {
        // Given a session actor where active persona "foo" was deleted from disk.
        let (actor, state, _audit) = create_actor().await;
        {
            let mut guard = state.write();
            guard.context.active_persona = Some(make_persona("foo"));
        }
        let personas = vec![make_persona("coding-assistant")];
        let payload = PersonasLoaded {
            personas,
            error: None,
        };

        // When receiving PersonasLoaded.
        actor.on_personas_loaded(&payload);

        // Then falls back to coding-assistant.
        let guard = state.read();
        assert_eq!(
            guard
                .context
                .active_persona
                .as_ref()
                .map(|p| p.name.as_str()),
            Some("coding-assistant")
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn on_personas_loaded_uses_first_when_coding_assistant_missing() {
        // Given a session actor with no coding-assistant in the scanned list.
        let (actor, state, _audit) = create_actor().await;
        let personas = vec![make_persona("learning-tutor")];
        let payload = PersonasLoaded {
            personas,
            error: None,
        };

        // When receiving PersonasLoaded.
        actor.on_personas_loaded(&payload);

        // Then first available is selected.
        let guard = state.read();
        assert_eq!(
            guard
                .context
                .active_persona
                .as_ref()
                .map(|p| p.name.as_str()),
            Some("learning-tutor")
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn on_personas_loaded_clears_active_when_list_empty() {
        // Given a session actor with some active persona.
        let (actor, state, _audit) = create_actor().await;
        {
            let mut guard = state.write();
            guard.context.active_persona = Some(make_persona("foo"));
        }
        let payload = PersonasLoaded {
            personas: vec![],
            error: None,
        };

        // When receiving PersonasLoaded with empty list.
        actor.on_personas_loaded(&payload);

        // Then active_persona is None.
        let guard = state.read();
        assert!(guard.context.active_persona.is_none());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_pin_chat_entry_pins_and_emits() {
        // Given a session with a user entry.
        let (actor, state, audit) = create_actor().await;
        let entry_id = {
            let mut guard = state.write();
            let session = guard.active_session_mut();
            let entry = crate::protocol::ChatEntry::user("hello");
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
            let mut entry = crate::protocol::ChatEntry::user("hello");
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

    /// Pushes `n` pinned entries (all `Top`, so display order = insertion order)
    /// into the active session and returns their IDs in insertion order.
    fn push_pinned_entries(state: &State, n: usize) -> Vec<ChatEntryId> {
        let mut guard = state.write();
        let session = guard.active_session_mut();
        (0..n)
            .map(|i| {
                let mut entry = crate::protocol::ChatEntry::user(format!("entry {i}"));
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
        state.write().frontend.pins.select_by_id(ids[0].clone());

        // When unpinning A.
        actor
            .handle_unpin_chat_entry(&UnpinChatEntry {
                session_id,
                entry_id: ids[0].clone(),
            })
            .await;

        // Then the cursor lands on B (now at index 0).
        assert_eq!(
            state.read().frontend.pins.selected_id().cloned(),
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
        state.write().frontend.pins.select_by_id(ids[1].clone());

        // When unpinning B.
        actor
            .handle_unpin_chat_entry(&UnpinChatEntry {
                session_id,
                entry_id: ids[1].clone(),
            })
            .await;

        // Then the cursor lands on C (now at index 1).
        assert_eq!(
            state.read().frontend.pins.selected_id().cloned(),
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
        state.write().frontend.pins.select_by_id(selected.clone());

        // When unpinning A (a different, non-selected entry).
        actor
            .handle_unpin_chat_entry(&UnpinChatEntry {
                session_id,
                entry_id: ids[0].clone(),
            })
            .await;

        // Then the cursor stays on B (its ID is still present).
        assert_eq!(
            state.read().frontend.pins.selected_id().cloned(),
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
        state.write().frontend.pins.select_by_id(ids[2].clone());

        // When unpinning C.
        actor
            .handle_unpin_chat_entry(&UnpinChatEntry {
                session_id,
                entry_id: ids[2].clone(),
            })
            .await;

        // Then the cursor clamps to B (the new last entry).
        assert_eq!(
            state.read().frontend.pins.selected_id().cloned(),
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
        state.write().frontend.pins.select_by_id(ids[0].clone());

        // When unpinning A.
        actor
            .handle_unpin_chat_entry(&UnpinChatEntry {
                session_id,
                entry_id: ids[0].clone(),
            })
            .await;

        // Then the cursor is cleared.
        assert!(state.read().frontend.pins.selected_id().is_none());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn unpin_for_non_active_session_leaves_cursor_unchanged() {
        // Given 3 pinned entries [A, B, C] with B selected, and an unrelated session.
        let (actor, state, _audit) = create_actor().await;
        let ids = push_pinned_entries(&state, 3);
        let selected = ids[1].clone();
        state.write().frontend.pins.select_by_id(selected.clone());

        // When unpinning B under a non-active session id.
        actor
            .handle_unpin_chat_entry(&UnpinChatEntry {
                session_id: SessionId::new(),
                entry_id: ids[1].clone(),
            })
            .await;

        // Then the cursor is unchanged.
        assert_eq!(
            state.read().frontend.pins.selected_id().cloned(),
            Some(selected)
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_load_persona_picker_entries_populates_picker() {
        // Given a session actor with personas loaded.
        let (actor, state, _audit) = create_actor().await;
        {
            let mut guard = state.write();
            guard.context.personas = vec![
                make_persona("coding-assistant"),
                make_persona("learning-tutor"),
            ];
            guard.context.active_persona = Some(make_persona("learning-tutor"));
        }

        // When loading persona picker entries.
        actor.handle_load_persona_picker_entries(&LoadPersonaPickerEntries);

        // Then the picker has entries with correct active state.
        let guard = state.read();
        let items = guard.frontend.persona_picker().items();
        assert_eq!(items.len(), 2, "expected 2 persona entries");
        let active = items.iter().find(|e| e.is_active).expect("an active entry");
        assert_eq!(active.name, "learning-tutor");
    }
}
