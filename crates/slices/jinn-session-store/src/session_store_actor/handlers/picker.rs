//! Session picker hydration from the store.

use jinn_session_store_msg::LoadSessionPickerEntries;

use crate::session_store_actor::SessionStoreActor;

impl SessionStoreActor {
    /// Loads session tree entries from storage into the session picker.
    pub(crate) async fn handle_load_session_picker_entries(
        &self,
        _payload: &LoadSessionPickerEntries,
    ) {
        let (store, theme) = {
            let state = self.state.read();
            (
                self.services.session_store.clone(),
                state.frontend.theme.clone(),
            )
        };
        let entries =
            jinn_domain::feat::session::entries::load_session_entries_from_store(&store, &theme)
                .await;
        // Into the picker's own cell, not the kernel's frontend state: the
        // picker is this slice's, and its rows are its state.
        self.session_picker_cell.update(|picker| {
            crate::session_picker_actions::load(picker, entries);
        });
    }
}
