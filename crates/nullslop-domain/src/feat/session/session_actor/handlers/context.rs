//! Context-related handlers — pinning, caching, and persona management.
//!
//! Handles entry pinning (PinChatEntry/UnpinChatEntry), tool definition caching
//! (ToolsRegistered), prompt template caching (PromptTemplatesLoaded),
//! persona selection (PersonasLoaded), and persona picker population
//! (LoadPersonaPickerEntries).
//!
//! Relocated from `PromptAssemblyActor` — these concerns are session-related
//! mutations of `AppState`, not part of prompt assembly.

use crate::common::actor::ActorContext;
use crate::feat::context::prompt_template::PromptTemplateStore;
use crate::feat::context::protocol::command::{
    LoadPersonaPickerEntries, PinChatEntry, UnpinChatEntry,
};
use crate::feat::context::protocol::event::ChatEntryPinChanged;
use crate::feat::persona::PersonaEntry;
use crate::feat::provider::protocol::event::PromptTemplatesLoaded;
use crate::feat::tools_actor::protocol::event::ToolsRegistered;
use crate::protocol::Event;

use super::super::SessionPersistenceActor;

impl SessionPersistenceActor {
    /// PinChatEntry: pin entry in session.
    pub(in crate::feat::session::session_actor) fn handle_pin_chat_entry(
        &self,
        payload: &PinChatEntry,
        ctx: &ActorContext,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.pin_entry(&payload.entry_id, payload.position);
        }
        let _ = ctx.send_event(Event::ChatEntryPinChanged(ChatEntryPinChanged {
            session_id: payload.session_id.clone(),
        }));
    }

    /// UnpinChatEntry: unpin entry in session.
    pub(in crate::feat::session::session_actor) fn handle_unpin_chat_entry(
        &self,
        payload: &UnpinChatEntry,
        ctx: &ActorContext,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.unpin_entry(&payload.entry_id);
        }
        let _ = ctx.send_event(Event::ChatEntryPinChanged(ChatEntryPinChanged {
            session_id: payload.session_id.clone(),
        }));
    }

    /// Caches tool definitions from a [`ToolsRegistered`] event into shared state.
    ///
    /// Judge-specific tools are excluded — they are injected into judge sessions
    /// during prompt assembly, not stored in the global tool map.
    pub(in crate::feat::session::session_actor) fn on_tools_registered(
        &self,
        evt: &ToolsRegistered,
    ) {
        let judge_names: std::collections::HashSet<String> =
            crate::feat::judge::judge_tool_definitions()
                .into_iter()
                .map(|d| d.name)
                .collect();

        let mut state = self.state.write();
        for def in &evt.definitions {
            if judge_names.contains(&def.name) {
                continue;
            }
            state
                .context
                .tool_definitions
                .insert(def.name.clone(), def.clone());
        }
    }

    /// Replaces the prompt template store with the loaded templates.
    pub(in crate::feat::session::session_actor) fn on_prompt_templates_loaded(
        &self,
        event: &PromptTemplatesLoaded,
    ) {
        let mut state = self.state.write();
        state.context.prompt_templates = PromptTemplateStore::from_vec(event.templates.clone());
    }

    /// Stores loaded personas in state and selects the active persona.
    ///
    /// Priority:
    /// 1. Honor prefs.persona_name if set and found in list.
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
            .preferences
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

    /// Stores loaded judges in state.
    pub(in crate::feat::session::session_actor) fn on_judges_loaded(
        &self,
        payload: &crate::feat::judge::JudgesLoaded,
    ) {
        if payload.error.is_some() {
            tracing::warn!(
                error = ?payload.error,
                "judge scan reported an error"
            );
            return;
        }
        let mut state = self.state.write();
        state.context.judges.clone_from(&payload.judges);
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

    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    let mut state = self.state.write();
        state.frontend.persona_picker.set_items(entries);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
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

    fn create_actor() -> (SessionPersistenceActor, State) {
        let sink: Arc<dyn MessageSink> = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("test", sink);
        let state = State::new(AppState::default());
        let deps = SessionPersistenceActorDeps {
            state: state.clone(),
            services: Some(TestServices::builder().build()),
            store: None,
            counter: crate::feat::context::strategy::token_estimator::TiktokenCounter::o200k_base(),
            builtin_registry: crate::feat::session_lifecycle::builtin::BuiltinRegistry::new(),
            shell: "/bin/sh".to_owned(),
        };
        let actor = SessionPersistenceActor::activate(deps, &mut ctx);
        (actor, state)
    }

    #[rstest::rstest]
    fn on_tools_registered_excludes_judge_tools_from_global_map() {
        // Given a session actor.
        let (actor, state) = create_actor();

        // Build a ToolsRegistered with all builtin tools (including judge tools).
        let all_tools = crate::feat::tools_actor::registry::builtin_tools();
        let definitions: Vec<_> = all_tools.iter().map(|(def, _)| def.clone()).collect();
        let payload = ToolsRegistered {
            provider: "builtin".to_owned(),
            definitions,
        };

        // When processing the event.
        actor.on_tools_registered(&payload);

        // Then no judge tool names are in the global map.
        let guard = state.read();
        assert!(
            !guard.context.tool_definitions.contains_key("judge_session_query"),
            "session_query should not be in global tool map"
        );
        assert!(
            !guard
                .context
                .tool_definitions
                .contains_key("judge_session_query_recent"),
            "session_query_recent should not be in global tool map"
        );
        assert!(
            !guard.context.tool_definitions.contains_key("judge_task_complete"),
            "task_complete should not be in global tool map"
        );
        assert!(
            !guard.context.tool_definitions.contains_key("judge_task_incomplete"),
            "task_incomplete should not be in global tool map"
        );
    }

    #[rstest::rstest]
    fn on_tools_registered_keeps_regular_tools_in_global_map() {
        // Given a session actor.
        let (actor, state) = create_actor();

        // Build a ToolsRegistered with all builtin tools.
        let all_tools = crate::feat::tools_actor::registry::builtin_tools();
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
    fn on_personas_loaded_selects_coding_assistant_when_none_active() {
        // Given a session actor with no active persona.
        let (actor, state) = create_actor();
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
    fn on_personas_loaded_keeps_existing_active_persona() {
        // Given a session actor with active persona "learning-tutor".
        let (actor, state) = create_actor();
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
    fn on_personas_loaded_falls_back_when_active_missing() {
        // Given a session actor where active persona "foo" was deleted from disk.
        let (actor, state) = create_actor();
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
    fn on_personas_loaded_uses_first_when_coding_assistant_missing() {
        // Given a session actor with no coding-assistant in the scanned list.
        let (actor, state) = create_actor();
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
    fn on_personas_loaded_clears_active_when_list_empty() {
        // Given a session actor with some active persona.
        let (actor, state) = create_actor();
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
    fn handle_pin_chat_entry_pins_and_emits() {
        // Given a session with a user entry.
        let (actor, state) = create_actor();
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
        actor.handle_pin_chat_entry(
            &PinChatEntry {
                session_id: session_id.clone(),
                entry_id: entry_id.clone(),
                position: PinPosition::Top,
            },
            &ctx,
        );

        // Then the entry is pinned.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");
        let entry = session.history().iter().find(|e| e.id == entry_id).expect("entry");
        assert!(entry.is_pinned(), "expected entry to be pinned");

        // And ChatEntryPinChanged event was emitted.
        let has_event = sink.events().iter().any(|e| {
            matches!(e, Event::ChatEntryPinChanged(e) if e.session_id == session_id)
        });
        assert!(has_event, "expected ChatEntryPinChanged event");
    }

    // --- handle_unpin_chat_entry ---

    #[rstest::rstest]
    fn handle_unpin_chat_entry_unpins_and_emits() {
        // Given a session with a pinned entry.
        let (actor, state) = create_actor();
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
        actor.handle_unpin_chat_entry(
            &UnpinChatEntry {
                session_id: session_id.clone(),
                entry_id: entry_id.clone(),
            },
            &ctx,
        );

        // Then the entry is no longer pinned.
        let guard = state.read();
        let session = guard.session.get(&session_id).expect("session");
        let entry = session.history().iter().find(|e| e.id == entry_id).expect("entry");
        assert!(!entry.is_pinned(), "expected entry to be unpinned");

        // And ChatEntryPinChanged event was emitted.
        let has_event = sink.events().iter().any(|e| {
            matches!(e, Event::ChatEntryPinChanged(e) if e.session_id == session_id)
        });
        assert!(has_event, "expected ChatEntryPinChanged event");
    }

    // --- on_prompt_templates_loaded ---

    #[rstest::rstest]
    fn on_prompt_templates_loaded_caches_templates() {
        // Given a session actor.
        let (actor, state) = create_actor();
        let templates = vec![
            crate::feat::context::protocol::prompt_template::PromptTemplate {
                name: "test-template".to_owned(),
                description: "A test".to_owned(),
                body: "Hello {{name}}".to_owned(),
            },
        ];

        // When loading prompt templates.
        actor.on_prompt_templates_loaded(&PromptTemplatesLoaded {
            templates: templates.clone(),
            error: None,
        });

        // Then the templates are stored in state.
        let guard = state.read();
        assert_eq!(guard.context.prompt_templates.len(), 1);
        assert_eq!(guard.context.prompt_templates.templates()[0].name, "test-template");
    }

    // --- on_judges_loaded ---

    #[rstest::rstest]
    fn on_judges_loaded_caches_judges() {
        // Given a session actor.
        let (actor, state) = create_actor();
        let judges = vec![crate::feat::judge::Judge {
            name: "test-judge".to_owned(),
            description: "A test judge".to_owned(),
            body: "Evaluate the output".to_owned(),
            model: None,
            auto_reset: false,
            file_path: std::path::PathBuf::from("/judges/test.md"),
        }];

        // When loading judges.
        actor.on_judges_loaded(&crate::feat::judge::JudgesLoaded {
            judges: judges.clone(),
            error: None,
        });

        // Then the judges are stored in state.
        let guard = state.read();
        assert_eq!(guard.context.judges.len(), 1);
        assert_eq!(guard.context.judges[0].name, "test-judge");
    }

    // --- handle_load_persona_picker_entries ---

    #[rstest::rstest]
    fn handle_load_persona_picker_entries_populates_picker() {
        // Given a session actor with personas loaded.
        let (actor, state) = create_actor();
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
        let items = guard.frontend.persona_picker.items();
        assert_eq!(items.len(), 2, "expected 2 persona entries");
        let active = items.iter().find(|e| e.is_active).expect("an active entry");
        assert_eq!(active.name, "learning-tutor");
    }
}
