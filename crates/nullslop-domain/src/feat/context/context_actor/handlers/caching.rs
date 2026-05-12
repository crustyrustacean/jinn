//! Caching handlers — tool definition caching and template loading.

use crate::feat::context::prompt_template::PromptTemplateStore;
use crate::feat::provider::protocol::event::PromptTemplatesLoaded;
use crate::feat::tools_actor::protocol::event::ToolsRegistered;

use super::super::PromptAssemblyActor;

impl PromptAssemblyActor {
    /// Caches tool definitions from a [`ToolsRegistered`] event.
    pub(in crate::feat::context::context_actor) fn on_tools_registered(
        &mut self,
        evt: &ToolsRegistered,
    ) {
        for def in &evt.definitions {
            self.tool_definitions.insert(def.name.clone(), def.clone());
        }
    }

    /// Replaces the prompt template store with the loaded templates.
    pub(in crate::feat::context::context_actor) fn on_prompt_templates_loaded(
        &self,
        event: &PromptTemplatesLoaded,
    ) {
        let mut state = self.state.write();
        state.context.prompt_templates = PromptTemplateStore::from_vec(event.templates.clone());
    }
}
