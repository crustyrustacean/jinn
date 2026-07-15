//! Host trait implementations on `StoreState`.
//!
//! The generated `Host` trait (from the `host` WIT interface) has one method
//! per import. The bag accessors are fully host-side (no domain coupling) and
//! are implemented here. The `emit`/`request-*`/`create-session`/`cancel-task`
//! methods route through injected callbacks (`HostImports`) carried in
//! `StoreState`; those callbacks are wired to the real domain services in
//! Phase 3 when `Services` constructs the WASM backend.

use crate::bindings::jinn::plugin::host::Host;
use crate::bindings::jinn::plugin::types::Command;
use crate::store::StoreState;

impl Host for StoreState {
    fn emit(&mut self, cmd: Command) {
        if let Some(ref imports) = self.imports {
            (imports.emit)(&self.ctx.plugin_name, &cmd, &self.ctx);
        }
    }

    fn cancel_task(&mut self, name: String) {
        if let Some(ref imports) = self.imports {
            (imports.cancel_task)(&name);
        }
    }

    fn get_plugin_data(&mut self) -> Option<Vec<u8>> {
        self.bags
            .get_for_session_ctx(&self.ctx)
            .or_else(|| self.bags.get(&self.ctx.plugin_name))
    }

    fn set_plugin_data(&mut self, data: Vec<u8>) {
        if let Some(session_id) = &self.ctx.session_id {
            self.bags
                .set_for_session(session_id, &self.ctx.instance_id, data);
        } else {
            self.bags.set(&self.ctx.plugin_name, data);
        }
    }

    fn get_global_data(&mut self, key: String) -> Option<Vec<u8>> {
        self.globals.get(&key)
    }

    fn set_global_data(&mut self, key: String, data: Option<Vec<u8>>) {
        match data {
            Some(bytes) => self.globals.set(&key, bytes),
            None => {
                self.globals.remove(&key);
            }
        }
    }
}
