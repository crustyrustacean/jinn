//! Host trait implementations on `StoreState`.
//!
//! The generated `Host` trait (from the `host` WIT interface) has one method
//! per import. The bag accessors are fully host-side (no domain coupling) and
//! are implemented here. The `emit`/`cancel-task` methods route through
//! injected callbacks (`HostImports`) carried in `StoreState`; those callbacks
//! are wired to the real domain services in Phase 3 when `Services` constructs
//! the WASM backend.
//!
//! The async imports (`request-llm-oneshot`, `create-session`) live on the
//! generated `HostWithStore<T>` trait — a separate trait for async functions,
//! whose methods receive an `Accessor` rather than `self`. The accessor grants
//! temporary access to the store data via `with(...)`.

use crate::bindings::command::{
    Command, CreateSessionReq, CreateSessionResp, LlmOneshotReq, LlmResp, RequestError,
};
use crate::bindings::jinn::plugin::host::{Host, HostWithStore};
use crate::store::StoreState;

impl Host for StoreState {
    async fn emit(&mut self, cmd: Command) -> wasmtime::Result<()> {
        tracing::debug!(plugin = %self.ctx.plugin_name, cmd = ?cmd, "host emit");
        if let Some(imports) = &self.imports {
            (imports.emit)(&self.ctx.plugin_name, &cmd, &self.ctx);
        }
        Ok(())
    }

    async fn cancel_task(&mut self, name: String) -> wasmtime::Result<()> {
        if let Some(imports) = &self.imports {
            (imports.cancel_task)(&name);
        }
        Ok(())
    }

    async fn get_plugin_data(&mut self) -> wasmtime::Result<Option<Vec<u8>>> {
        let v = self.bags.get_for_session_ctx(&self.ctx);
        tracing::debug!(plugin = %self.ctx.plugin_name, session = ?self.ctx.session_id, bytes = ?v.as_deref(), "get_plugin_data");
        Ok(v)
    }

    async fn set_plugin_data(&mut self, data: Vec<u8>) -> wasmtime::Result<()> {
        tracing::debug!(plugin = %self.ctx.plugin_name, session = ?self.ctx.session_id, len = data.len(), "set_plugin_data");
        match &self.ctx.session_id {
            Some(sid) => self.bags.set_for_session(sid, &self.ctx.instance_id, data),
            None => self.bags.set(&self.ctx.plugin_name, data),
        }
        Ok(())
    }

    async fn get_global_data(&mut self, key: String) -> wasmtime::Result<Option<Vec<u8>>> {
        Ok(self.globals.get(&key))
    }

    async fn set_global_data(
        &mut self,
        key: String,
        data: Option<Vec<u8>>,
    ) -> wasmtime::Result<()> {
        match data {
            Some(bytes) => self.globals.set(&key, bytes),
            None => {
                self.globals.remove(&key);
            }
        }
        Ok(())
    }
}

impl HostWithStore<StoreState> for StoreState {
    async fn request_llm_oneshot(
        accessor: &wasmtime::component::Accessor<StoreState, StoreState>,
        req: LlmOneshotReq,
    ) -> wasmtime::Result<Result<LlmResp, RequestError>> {
        // Snapshot the immutable fields we need out of the store data so the
        // async future does not borrow the accessor (it can't span an `.await`).
        let (imports, ctx) =
            accessor.with(|mut access| (access.get().imports.clone(), access.get().ctx.clone()));
        match imports {
            Some(imports) => Ok((imports.llm_oneshot)(&ctx, &req).await),
            None => Ok(Err(RequestError::Other("no host imports wired".into()))),
        }
    }

    async fn create_session(
        accessor: &wasmtime::component::Accessor<StoreState, StoreState>,
        req: CreateSessionReq,
    ) -> wasmtime::Result<Result<CreateSessionResp, RequestError>> {
        let (imports, ctx) =
            accessor.with(|mut access| (access.get().imports.clone(), access.get().ctx.clone()));
        match imports {
            Some(imports) => Ok((imports.create_session)(&ctx, &req).await),
            None => Ok(Err(RequestError::Other("no host imports wired".into()))),
        }
    }
}
