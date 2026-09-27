pub mod chat_input_state;
pub mod command;
pub mod event;
mod file_picker_state;
pub mod scope;
pub mod slash_command;

pub use chat_input_state::*;
pub use command::{EnqueueResumeTurn, EnqueueUserMessage, ListDirectory, SubmitSteeringMessage};
pub use event::ChatEntrySubmitted;
pub use file_picker_state::{FileEntry, FilePickerState, resolve_list_dir};
pub use scope::chat_input_scope;
pub use slash_command::{SlashCommand, SlashCommandEntry};

#[cfg(test)]
mod tests {
    use super::*;
    use trouper::schema::{Schema, SchemaKind};

    #[rstest::rstest]
    #[test]
    fn crossing_schema_contract_is_stable() {
        // Given the canonical chat-input crossing contracts.
        // When reading their schema definitions.
        let schemas = [
            <EnqueueUserMessage as Schema>::schema_def(),
            <EnqueueResumeTurn as Schema>::schema_def(),
            <SubmitSteeringMessage as Schema>::schema_def(),
            <ChatEntrySubmitted as Schema>::schema_def(),
            <ListDirectory as Schema>::schema_def(),
        ];

        // Then their names and command/event kinds are stable.
        assert_eq!(schemas[0].name, "EnqueueUserMessage");
        assert!(matches!(schemas[0].kind, SchemaKind::Command));
        assert_eq!(schemas[1].name, "EnqueueResumeTurn");
        assert!(matches!(schemas[1].kind, SchemaKind::Command));
        assert_eq!(schemas[2].name, "SubmitSteeringMessage");
        assert!(matches!(schemas[2].kind, SchemaKind::Command));
        assert_eq!(schemas[3].name, "ChatEntrySubmitted");
        assert!(matches!(schemas[3].kind, SchemaKind::Event));
        assert_eq!(schemas[4].name, "ListDirectory");
        assert!(matches!(schemas[4].kind, SchemaKind::Command));
    }
}
