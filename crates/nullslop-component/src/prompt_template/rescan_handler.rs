//! Prompt template rescan handler.
//!
//! Handles [`RescanPromptTemplates`] (posts status message) and
//! [`PromptTemplatesLoaded`] (updates `AppState`, posts summary).

use nullslop_component_core::{HandlerContext, define_handler};
use nullslop_protocol as npr;
use nullslop_protocol::CommandAction;
use nullslop_protocol::provider::{PromptTemplatesLoaded, RescanPromptTemplates};
use nullslop_prompt_template::PromptTemplateStore;
use nullslop_services::Services;

use crate::AppState;

define_handler! {
    pub(crate) struct RescanHandler;

    commands {
        RescanPromptTemplates: on_rescan_prompt_templates,
    }

    events {
        PromptTemplatesLoaded: on_prompt_templates_loaded,
    }
}

impl RescanHandler {
    /// Posts a "Rescanning prompt templates..." system message to the active session.
    fn on_rescan_prompt_templates(
        _cmd: &RescanPromptTemplates,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) -> CommandAction {
        ctx.state
            .active_session_mut()
            .push_entry(npr::ChatEntry::system("Rescanning prompt templates..."));
        CommandAction::Continue
    }

    /// Updates `AppState::prompt_templates` with the loaded results and posts a summary.
    ///
    /// On error, preserves the existing store and posts a failure message.
    fn on_prompt_templates_loaded(
        evt: &PromptTemplatesLoaded,
        ctx: &mut HandlerContext<'_, AppState, Services>,
    ) {
        if let Some(ref error) = evt.error {
            ctx.state
                .active_session_mut()
                .push_entry(npr::ChatEntry::system(format!(
                    "Failed to reload prompt templates: {error}"
                )));
            return;
        }

        let store = PromptTemplateStore::from_vec(evt.templates.clone());
        let count = store.len();
        ctx.state.prompt_templates = store;
        ctx.state
            .active_session_mut()
            .push_entry(npr::ChatEntry::system(format!(
                "Prompt templates reloaded: {count} templates"
            )));
    }
}

#[cfg(test)]
mod tests {
    use nullslop_component_core::Bus;
    use nullslop_protocol::Command;
    use nullslop_protocol::Event;
    use nullslop_protocol::provider::PromptTemplatesLoaded;
    use nullslop_services::Services;

    use super::*;
    use crate::test_utils;

    fn make_services() -> Services {
        test_utils::test_services()
    }

    #[rstest::rstest]    fn rescan_command_posts_status_message() {
        // Given a bus with RescanHandler registered.
        let mut bus: Bus<AppState, Services> = Bus::new();
        RescanHandler.register(&mut bus);

        // When processing a RescanPromptTemplates command.
        bus.submit_command(Command::RescanPromptTemplates);
        let services = make_services();
        let mut state = AppState::default();
        bus.process_commands(&mut state, &services);

        // Then a "Rescanning..." system message was posted.
        let entries = &state.active_session().history();
        let msg = entries.last().expect("at least one entry");
        match &msg.kind {
            npr::ChatEntryKind::System(text) => {
                assert!(text.contains("Rescanning"));
            }
            other => panic!("expected system message, got {other:?}"),
        }
    }

    #[rstest::rstest]    fn prompt_templates_loaded_updates_state() {
        // Given a bus with RescanHandler registered.
        let mut bus: Bus<AppState, Services> = Bus::new();
        RescanHandler.register(&mut bus);

        // When processing a PromptTemplatesLoaded event with templates.
        let templates = vec![npr::PromptTemplate {
            name: "test".to_owned(),
            description: "Test".to_owned(),
            body: "Hello".to_owned(),
        }];
        bus.submit_event(Event::PromptTemplatesLoaded {
            payload: PromptTemplatesLoaded {
                templates,
                error: None,
            },
        });
        let services = make_services();
        let mut state = AppState::default();
        bus.process_events(&mut state, &services);

        // Then the state store contains the template.
        assert_eq!(state.prompt_templates.len(), 1);
        assert!(state.prompt_templates.find_by_name("test").is_some());
    }

    #[rstest::rstest]    fn prompt_templates_loaded_posts_summary() {
        // Given a bus with RescanHandler registered.
        let mut bus: Bus<AppState, Services> = Bus::new();
        RescanHandler.register(&mut bus);

        // When processing a PromptTemplatesLoaded event.
        bus.submit_event(Event::PromptTemplatesLoaded {
            payload: PromptTemplatesLoaded {
                templates: vec![npr::PromptTemplate {
                    name: "a".to_owned(),
                    description: String::new(),
                    body: String::new(),
                }],
                error: None,
            },
        });
        let services = make_services();
        let mut state = AppState::default();
        bus.process_events(&mut state, &services);

        // Then a summary message was posted.
        let entries = &state.active_session().history();
        let msg = entries.last().expect("at least one entry");
        match &msg.kind {
            npr::ChatEntryKind::System(text) => {
                assert!(text.contains("1 templates"));
            }
            other => panic!("expected system message, got {other:?}"),
        }
    }

    #[rstest::rstest]    fn prompt_templates_loaded_error_preserves_state() {
        // Given a bus with RescanHandler registered and an existing template.
        let mut bus: Bus<AppState, Services> = Bus::new();
        RescanHandler.register(&mut bus);
        let mut state = AppState {
            prompt_templates: PromptTemplateStore::from_vec(vec![npr::PromptTemplate {
                name: "existing".to_owned(),
                description: String::new(),
                body: "Body".to_owned(),
            }]),
            ..AppState::default()
        };

        // When processing a PromptTemplatesLoaded event with an error.
        bus.submit_event(Event::PromptTemplatesLoaded {
            payload: PromptTemplatesLoaded {
                templates: vec![],
                error: Some("scan failed".to_owned()),
            },
        });
        let services = make_services();
        bus.process_events(&mut state, &services);

        // Then the old store is preserved.
        assert_eq!(state.prompt_templates.len(), 1);
        assert!(state.prompt_templates.find_by_name("existing").is_some());
    }
}
