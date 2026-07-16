//! Shared WASM engine — configured once, shared across all stores.
//!
//! `wasmtime::Engine` is `Send + Sync` and cheaply cloneable. `Store` is
//! `!Send`, which is why the dual-store model exists (see `store.rs`).
//!
//! Async support is enabled so host imports can suspend the component while
//! the host drives an async `Future` (the coroutine idiom: a plugin calls
//! `request-llm-oneshot`, the component's stack is parked until the LLM
//! stream resolves, then it resumes).

use error_stack::ResultExt;
use std::sync::Arc;
use wasmtime::Config;

/// Error constructing the WASM engine.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub struct EngineConfigError;

/// Configuration knobs for the engine.
///
///
/// Async support is enabled via `Config::async_support(true)` +
/// `async_stack_size`; the WIT marks async imports/hooks with the `async`
/// keyword, and bindgen generates the corresponding async trait methods.
/// Cranelift is the default (no JIT preemption needed for a trusted single-user TUI).
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Max stack bytes per async store. LLM tool loops can recurse; default
    /// is generous.
    pub async_stack_size: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            async_stack_size: 2 << 20, // 2 MiB
        }
    }
}

/// A shared, cloneable handle to the configured wasmtime engine.
///
/// Cheap to clone (internally `Arc`). All stores created from one engine can
/// share compiled modules.
#[derive(Debug, Clone)]
pub struct WasmEngine {
    engine: wasmtime::Engine,
}

impl WasmEngine {
    /// Build the engine from the given config.
    ///
    /// # Errors
    ///
    /// Returns an error if wasmtime fails to construct the engine (e.g. the
    /// platform rejects the configured async stack size).
    pub fn new(config: &EngineConfig) -> Result<Self, error_stack::Report<EngineConfigError>> {
        let mut cfg = Config::new();
        // Async support is built into the component model's `async func` WIT
        // syntax; wasmtime 46 enables it automatically. The deprecated
        // `async_support` toggle is a no-op.
        cfg.async_stack_size(config.async_stack_size);
        // The component model is on by default; set explicitly for clarity.
        cfg.wasm_component_model(true);
        let engine = wasmtime::Engine::new(&cfg)
            .map_err(|e| error_stack::Report::new(EngineConfigError).attach(e.to_string()))
            .attach("constructing wasmtime engine")?;
        Ok(Self { engine })
    }

    /// Borrow the underlying wasmtime engine for store/module creation.
    pub(crate) fn inner(&self) -> &wasmtime::Engine {
        &self.engine
    }
}

/// A component module compiled once and instantiable in any store built from
/// the same engine.
#[derive(Clone)]
pub struct CompiledComponent {
    component: Arc<wasmtime::component::Component>,
}

impl std::fmt::Debug for CompiledComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledComponent").finish_non_exhaustive()
    }
}

impl CompiledComponent {
    /// Compile raw `.wasm` bytes against the engine.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes are not a valid WASM component.
    pub fn compile(
        engine: &WasmEngine,
        bytes: &[u8],
    ) -> Result<Self, error_stack::Report<EngineConfigError>> {
        let component = wasmtime::component::Component::new(engine.inner(), bytes)
            .map_err(|e| error_stack::Report::new(EngineConfigError).attach(e.to_string()))
            .attach("compiling wasm component")?;
        Ok(Self {
            component: Arc::new(component),
        })
    }

    pub(crate) fn inner(&self) -> &wasmtime::component::Component {
        &self.component
    }
}
