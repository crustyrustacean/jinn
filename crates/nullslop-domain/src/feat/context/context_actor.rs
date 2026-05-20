//! Context actor — prompt assembly, pinning, and templates.
//!
//! Owns the full context/prompt domain: assembles LLM-ready prompts from chat
//! history using compaction, handles entry pinning, and loads prompt templates.
//! Subscribes to [`AssemblePrompt`], [`PinChatEntry`], [`UnpinChatEntry`] commands
//! and [`ToolsRegistered`], [`PromptTemplatesLoaded`] events.

mod handlers;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::context::protocol::command::{
    AssemblePrompt, LoadPersonaPickerEntries, PinChatEntry, UnpinChatEntry,
};
use crate::feat::context::protocol::event::PersonasLoaded;
use crate::feat::persona::PersonaEntry;
use crate::feat::provider::protocol::event::PromptTemplatesLoaded;
use crate::feat::tools_actor::protocol::event::ToolsRegistered;
use crate::protocol::{Command, Event};

/// The context actor — handles prompt assembly, pinning, and templates.
pub struct PromptAssemblyActor {
    /// Shared application state.
    pub(super) state: State,
    /// Runtime services.
    #[expect(dead_code, reason = "will be used for picker entry loading")]
    pub(super) services: Services,
}

/// Dependencies for [`PromptAssemblyActor`].
pub struct PromptAssemblyActorDeps {
    /// Shared application state.
    pub state: State,
    /// Runtime services.
    pub services: Services,
}

impl Actor for PromptAssemblyActor {
    type Message = NoDirectMsg;
    type Deps = PromptAssemblyActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<AssemblePrompt>();
        ctx.subscribe_event::<ToolsRegistered>();
        ctx.subscribe_event::<PersonasLoaded>();
        ctx.subscribe_command::<PinChatEntry>();
        ctx.subscribe_command::<UnpinChatEntry>();
        ctx.subscribe_command::<LoadPersonaPickerEntries>();
        ctx.subscribe_event::<PromptTemplatesLoaded>();

        ctx.set_description("Context assembly, pinning, and templates");

        Self {
            state: deps.state,
            services: deps.services,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(cmd) => {
                self.handle_command(&cmd, ctx).await;
            }
            ActorEnvelope::Event(evt) => {
                self.handle_event(&evt);
            }
            _ => {}
        }
    }
}

impl PromptAssemblyActor {
    /// Dispatches incoming commands to the appropriate handler.
    async fn handle_command(&mut self, cmd: &Command, ctx: &ActorContext) {
        match cmd {
            Command::AssemblePrompt(payload) => {
                self.on_assemble_prompt(payload, ctx).await;
            }
            Command::PinChatEntry(payload) => {
                self.handle_pin_chat_entry(payload, ctx);
            }
            Command::UnpinChatEntry(payload) => {
                self.handle_unpin_chat_entry(payload, ctx);
            }
            Command::LoadPersonaPickerEntries(payload) => {
                self.handle_load_persona_picker_entries(payload);
            }
            _ => {}
        }
    }

    /// Dispatches incoming events to the appropriate handler.
    fn handle_event(&mut self, evt: &Event) {
        match evt {
            Event::ToolsRegistered(payload) => {
                self.on_tools_registered(payload);
            }
            Event::PromptTemplatesLoaded(payload) => {
                self.on_prompt_templates_loaded(payload);
            }
            Event::PersonasLoaded(payload) => {
                self.on_personas_loaded(payload);
            }
            _ => {}
        }
    }

    /// Loads persona picker entries into `AppState`.
    fn handle_load_persona_picker_entries(&self, _payload: &LoadPersonaPickerEntries) {
        let state = self.state.read();
        let active_name = state
            .context
            .active_persona
            .as_ref()
            .map(|p| p.name.clone());
        let entries: Vec<PersonaEntry> = state
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

        let mut state = self.state.write();
        state.frontend.persona_picker.set_items(entries);
    }

    /// Stores loaded personas in state and selects the active persona.
    ///
    /// Priority:
    /// 1. Keep current `active_persona` if it still exists in the new list.
    /// 2. Fallback to `"coding-assistant"` by name lookup.
    /// 3. If coding-assistant not found, pick first available.
    fn on_personas_loaded(&self, payload: &PersonasLoaded) {
        if payload.error.is_some() {
            tracing::warn!(
                error = ?payload.error,
                "persona scan reported an error"
            );
            return;
        }
        let mut state = self.state.write();
        state.context.personas.clone_from(&payload.personas);

        // Priority:
        // 1. Honor prefs.persona_name if set and found in list.
        // 2. Keep current active_persona if it still exists in the new list.
        // 3. Fallback to coding-assistant by name.
        // 4. First available.
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
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use std::sync::Arc;

    use crate::common::actor::{Actor as _, ActorContext, MessageSink, RecordingSink};
    use crate::common::app_state::AppState;
    use crate::common::services::test_services::TestServices;
    use crate::common::state::State;
    use crate::feat::context::protocol::event::PersonasLoaded;
    use crate::feat::persona::Persona;

    use super::*;

    fn make_persona(name: &str) -> Persona {
        Persona {
            name: name.to_owned(),
            description: String::new(),
            body: String::new(),
            file_path: std::path::PathBuf::from(format!("/personas/{name}.md")),
        }
    }

    fn create_actor() -> (PromptAssemblyActor, State) {
        let sink: Arc<dyn MessageSink> = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("test", sink);
        let state = State::new(AppState::default());
        let deps = PromptAssemblyActorDeps {
            state: state.clone(),
            services: TestServices::builder().build(),
        };
        let actor = PromptAssemblyActor::activate(deps, &mut ctx);
        (actor, state)
    }

    #[rstest::rstest]
    fn on_personas_loaded_selects_coding_assistant_when_none_active() {
        // Given a context actor with no active persona.
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
        // Given a context actor with active persona "learning-tutor".
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
        // Given a context actor where active persona "foo" was deleted from disk.
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
        // Given a context actor with no coding-assistant in the scanned list.
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
        // Given a context actor with some active persona.
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
}
