//! Canonical contracts and shared vocabulary for the persona slice.

mod command;
mod event;
pub mod persona_entry;
pub mod persona_state;

pub use command::LoadPersonaPickerEntries;
pub use event::PersonasLoaded;
pub use persona_entry::{PersonaEntry, persona_row};
pub use persona_state::*;

#[cfg(test)]
mod tests {
    use super::*;
    use trouper::schema::{Schema, SchemaKind};

    #[rstest::rstest]
    #[test]
    fn crossing_schema_contract_is_stable() {
        // Given the canonical persona crossing contracts.
        // When reading their schema definitions.
        let command = <LoadPersonaPickerEntries as Schema>::schema_def();
        let event = <PersonasLoaded as Schema>::schema_def();

        // Then their names and command/event kinds are stable.
        assert_eq!(command.name, "LoadPersonaPickerEntries");
        assert!(matches!(command.kind, SchemaKind::Command));
        assert_eq!(event.name, "PersonasLoaded");
        assert!(matches!(event.kind, SchemaKind::Event));
    }
}
