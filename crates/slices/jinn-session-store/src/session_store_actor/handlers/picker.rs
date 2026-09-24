//! Session picker hydration from the store.

use jinn_domain::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
use jinn_domain::feat::ui::picker_states::PickerExt;

use crate::session_store_actor::SessionStoreActor;

impl SessionStoreActor {
    /// Loads session tree entries from storage into the session picker.
    pub(crate) async fn handle_load_session_picker_entries(
        &self,
        _payload: &LoadSessionPickerEntries,
    ) {
        let (store, theme) = {
            let state = self.state.read();
            (self.services.session_store.clone(), state.frontend.theme.clone())
        };
        let entries =
            jinn_domain::feat::session::entries::load_session_entries_from_store(&store, &theme)
                .await;
        let wrapped = jinn_picker::make_items_with_hooks(
            entries,
            jinn_picker::PickerItemHooks::new()
                .row(jinn_domain::feat::session::picker_entry::session_row)
                .search(
                    |entry: &jinn_domain::feat::session::picker_entry::SessionTreeEntry| {
                        entry.title.clone()
                    },
                ),
        );
        self.state.with_preferences(&self.frontend_cap, |ops| {
            ops.frontend().session_picker_mut().set_items(wrapped);
        });
    }
}
