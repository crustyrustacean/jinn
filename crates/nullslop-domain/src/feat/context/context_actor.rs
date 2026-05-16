//! Context actor — prompt assembly, strategy management, pinning, and templates.
//!
//! Owns the full context/prompt domain: assembles LLM-ready prompts from chat
//! history, manages prompt strategies, handles entry pinning, and loads prompt
//! templates. Subscribes to [`AssemblePrompt`], [`SwitchPromptStrategy`],
//! [`RestoreStrategyState`], [`PinChatEntry`], [`UnpinChatEntry`] commands and
//! [`PromptStrategySwitched`], [`ToolsRegistered`], [`PromptTemplatesLoaded`] events.
//!
//! Unknown sessions are automatically initialized with `PassthroughStrategy`.
//! Strategy switching uses a [`StrategyFactory`] injected via [`ActorContext`] data.

mod handlers;

use std::collections::HashMap;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::context::protocol::command::{
    AssemblePrompt, LoadContextStrategyPickerEntries, LoadPersonaPickerEntries, PinChatEntry,
    RestoreStrategyState, SwitchPromptStrategy, UnpinChatEntry,
};
use crate::feat::context::protocol::event::{PersonasLoaded, PromptStrategySwitched};
use crate::feat::persona::PersonaEntry;
use crate::feat::picker::strategy_entries::load_strategy_picker_items;
use crate::feat::provider::protocol::event::PromptTemplatesLoaded;
use crate::feat::tools_actor::protocol::event::ToolsRegistered;
use crate::protocol::{Command, Event, SessionId};

use crate::feat::context::{DefaultStrategyFactory, PromptAssembly, StrategyFactory};

/// The context actor — handles prompt assembly, strategy management, pinning, and templates.
pub struct PromptAssemblyActor {
    /// Shared application state.
    pub(super) state: State,
    /// Per-session prompt assembly strategies.
    pub(super) strategies: HashMap<SessionId, Box<dyn PromptAssembly>>,
    /// Factory for creating new strategies on switch.
    pub(super) factory: Option<Box<dyn StrategyFactory>>,
    /// Runtime services (strategy registry for picker loading).
    pub(super) services: Services,
}

impl Actor for PromptAssemblyActor {
    type Message = NoDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        // Existing subscriptions (prompt assembly).
        ctx.subscribe_command::<AssemblePrompt>();
        ctx.subscribe_event::<PromptStrategySwitched>();
        ctx.subscribe_event::<ToolsRegistered>();
        ctx.subscribe_event::<PersonasLoaded>();

        // New subscriptions (strategy management, pinning, templates, picker).
        ctx.subscribe_command::<SwitchPromptStrategy>();
        ctx.subscribe_command::<RestoreStrategyState>();
        ctx.subscribe_command::<PinChatEntry>();
        ctx.subscribe_command::<UnpinChatEntry>();
        ctx.subscribe_command::<LoadContextStrategyPickerEntries>();
        ctx.subscribe_command::<LoadPersonaPickerEntries>();
        ctx.subscribe_event::<PromptTemplatesLoaded>();

        ctx.set_description("Context assembly, strategy management, pinning, and templates");

        #[expect(clippy::expect_used, reason = "State is always injected at startup")]
        let state = ctx
            .take_data::<State>()
            .expect("PromptAssemblyActor requires State injection");
        let factory = ctx
            .take_data::<Box<dyn StrategyFactory>>()
            .unwrap_or_else(|| Box::new(DefaultStrategyFactory));
        #[expect(clippy::expect_used, reason = "Services is always injected at startup")]
        let services = ctx
            .take_data::<Services>()
            .expect("PromptAssemblyActor requires Services injection");
        Self {
            state,
            strategies: HashMap::new(),
            factory: Some(factory),
            services,
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
                self.handle_pin_chat_entry(payload);
            }
            Command::UnpinChatEntry(payload) => {
                self.handle_unpin_chat_entry(payload);
            }
            Command::SwitchPromptStrategy(payload) => {
                self.handle_switch_prompt_strategy(payload, ctx);
            }
            Command::RestoreStrategyState(payload) => {
                self.handle_restore_strategy_state(payload, ctx);
            }
            Command::LoadContextStrategyPickerEntries(payload) => {
                self.handle_load_context_strategy_picker_entries(payload);
            }
            Command::LoadPersonaPickerEntries(payload) => {
                self.handle_load_persona_picker_entries(payload);
            }
            // RescanPersonas is handled by persona-scan actor.
            _ => {}
        }
    }

    /// Dispatches incoming events to the appropriate handler.
    fn handle_event(&mut self, evt: &Event) {
        match evt {
            Event::ToolsRegistered(payload) => {
                self.on_tools_registered(payload);
            }
            Event::PromptStrategySwitched(payload) => {
                self.on_prompt_strategy_switched(payload);
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

    /// Loads context strategy picker entries into `AppState`.
    fn handle_load_context_strategy_picker_entries(
        &self,
        _payload: &LoadContextStrategyPickerEntries,
    ) {
        let mut state = self.state.write();
        load_strategy_picker_items(&self.services, &mut state);
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

    /// Stores loaded personas in state and sets the first as default if none active.
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
        if state.context.active_persona.is_none() {
            state.context.active_persona = payload.personas.first().cloned();
        }
    }
}
