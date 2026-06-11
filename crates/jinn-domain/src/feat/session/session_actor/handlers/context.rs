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
            let session = state.session_mut_or_create(&payload.session_id);
            session.unpin_entry(&payload.entry_id);
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
        for def in &evt.definitions {
            state
                .context
                .tool_definitions
                .insert(def.name.clone(), def.clone());
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
    pub(in crate::feat::session::session_actor) async fn handle_load_persona_picker_entries(
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

        entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

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
    use std::sync::Arc;

    use crate::common::actor::{Actor as _, ActorContext, MessageSink, RecordingSink};
    use crate::common::app_state::AppState;
    use crate::common::services::test_services::TestServices;
    use crate::common::state::State;
    use crate::feat::context::protocol::event::PersonasLoaded;
    use crate::feat::persona::Persona;

    use super::super::super::SessionPersistenceActorDeps;
    use super::*;
    use crate::protocol::PinPosition;

    fn make_persona(name: &str) -> Persona {
        Persona {
            name: name.to_owned(),
            description: String::new(),
            body: String::new(),
            file_path: std::path::PathBuf::from(format!("/personas/{name}.md")),
        }
    }

    async fn create_actor() -> (SessionPersistenceActor, State) {
        let state = State::new(AppState::default());
        let actor = SessionPersistenceActor {
            state: state.clone(),
            services: crate::common::services::Services::new_fake().await,
            counter: crate::feat::context::strategy::token_estimator::TiktokenCounter::o200k_base(),
            builtin_registry: crate::feat::session_lifecycle::builtin::BuiltinRegistry::new(),
            shell: "/bin/sh".to_owned(),
            lifecycle_child: None,
        };
        (actor, state)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn on_tools_registered_keeps_regular_tools_in_global_map() {
        // Given a session actor.
        let (actor, state) = create_actor().await;

        // Build a ToolsRegistered with all builtin tools.
        let all_tools = crate::feat::tools_actor::registry::builtin_tools(
            &crate::feat::preferences_actor::user_preferences::BashConfig::default(),
        );
        let definitions: Vec<_> = all_tools.iter().map(|(def, _)| def.clone()).collect();
        let payload = ToolsRegistered {
            provider: "builtin".to_owned(),
            definitions,
        };

        // When processing the event.
        actor.on_tools_registered(&payload);

        // Then regular tools are in the global map.
        let guard = state.read();
        assert!(
            guard.context.tool_definitions.contains_key("bash"),
            "bash should be in global tool map"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn on_personas_loaded_selects_coding_assistant_when_none_active() {
        // Given a session actor with no active persona.
        let (actor, state) = create_actor().await;
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
        let (actor, state) = create_actor().await;
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
        let (actor, state) = create_actor().await;
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
        let (actor, state) = create_actor().await;
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
        let (actor, state) = create_actor().await;
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

    // --- handle_pin_chat_entry ---

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_pin_chat_entry_pins_and_emits() {
        // Given a session with a user entry.
        let (actor, state) = create_actor().await;
        let (sink, ctx) = {
            let sink = Arc::new(RecordingSink::new());
            let ctx = ActorContext::new("test", sink.clone());
            (sink, ctx)
        };
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
        actor.handle_pin_chat_entry(&PinChatEntry {
            session_id: session_id.clone(),
            entry_id: entry_id.clone(),
            position: PinPosition::Top,
        });

        // Then the entry is pinned.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");
        let entry = session
            .history()
            .iter()
            .find(|e| e.id == entry_id)
            .expect("entry");
        assert!(entry.is_pinned(), "expected entry to be pinned");

        // And ChatEntryPinChanged event was emitted.
        let has_event = sink
            .events()
            .iter()
            .any(|e| matches!(e, Event::ChatEntryPinChanged(e) if e.session_id == session_id));
        assert!(has_event, "expected ChatEntryPinChanged event");
    }

    // --- handle_unpin_chat_entry ---

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_unpin_chat_entry_unpins_and_emits() {
        // Given a session with a pinned entry.
        let (actor, state) = create_actor().await;
        let (sink, ctx) = {
            let sink = Arc::new(RecordingSink::new());
            let ctx = ActorContext::new("test", sink.clone());
            (sink, ctx)
        };
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
        actor.handle_unpin_chat_entry(&UnpinChatEntry {
            session_id: session_id.clone(),
            entry_id: entry_id.clone(),
        });

        // Then the entry is no longer pinned.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");
        let entry = session
            .history()
            .iter()
            .find(|e| e.id == entry_id)
            .expect("entry");
        assert!(!entry.is_pinned(), "expected entry to be unpinned");

        // And ChatEntryPinChanged event was emitted.
        let has_event = sink
            .events()
            .iter()
            .any(|e| matches!(e, Event::ChatEntryPinChanged(e) if e.session_id == session_id));
        assert!(has_event, "expected ChatEntryPinChanged event");
    }

    // --- on_prompt_templates_loaded ---

    // --- handle_load_persona_picker_entries ---

    #[rstest::rstest]
    #[tokio::test]
    async fn handle_load_persona_picker_entries_populates_picker() {
        // Given a session actor with personas loaded.
        let (actor, state) = create_actor().await;
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
