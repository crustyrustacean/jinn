//! Context capsule: cap + view + extension traits, colocated.
//!
//! Write access to [`ContextAssemblyState`] is gated by an unforgeable ZST token
//! ([`ContextCap`]). The projection method [`State::with_context`] hands the
//! cap-holder a narrow borrowed view ([`ContextView`]).
//!
//! The context capsule spans personas, global/per-session tool definitions, and
//! the compaction prompt. Extension traits partition the write surface by
//! concern so each owner opts into exactly what it writes.

use std::collections::BTreeMap;

use crate::common::state::State;
use crate::feat::context::assembly_state::ContextAssemblyState;
use crate::feat::persona::Persona;
use crate::protocol::SessionId;
use crate::protocol::ToolDefinition;

// ── The cap ──────────────────────────────────────────────────────────────────

/// Proof of authority to write [`ContextAssemblyState`]. Minted only via
/// [`crate::common::tcaps::mint`].
#[derive(Clone, Copy, Debug)]
pub struct ContextCap(());

impl ContextCap {
    /// Private constructor scoped to the `tcaps/` subtree.
    pub(in crate::common::tcaps) fn new() -> Self {
        Self(())
    }
}

// ── Per-struct narrow newtype ───────────────────────────────────────────────

/// Narrow write-handle to [`ContextAssemblyState`]. The tuple field is PRIVATE.
pub struct ContextOps<'a>(&'a mut ContextAssemblyState);

// ── Composite facade ─────────────────────────────────────────────────────────

/// What a context-writer sees: mutable access to the context assembly state,
/// scoped via [`ContextOps`].
pub struct ContextView<'a> {
    /// Mutable context state, scoped via [`ContextOps`].
    pub context: ContextOps<'a>,
}

// ── Extension traits (the opt-in method menu) ───────────────────────────────

/// Write access to personas.
pub trait PersonaWrite {
    fn set_active_persona(&mut self, persona: Option<Persona>);
    fn set_personas(&mut self, personas: Vec<Persona>);
    fn active_persona(&self) -> Option<&Persona>;
}

/// Write access to global tool definitions.
pub trait GlobalToolDefinitionsWrite {
    fn global_tool_definitions_mut(&mut self) -> &mut BTreeMap<String, ToolDefinition>;
}

/// Write access to per-session tool definitions.
pub trait SessionToolDefinitionsWrite {
    fn session_tool_definitions_mut(
        &mut self,
    ) -> &mut BTreeMap<SessionId, BTreeMap<String, ToolDefinition>>;
}

impl PersonaWrite for ContextOps<'_> {
    fn set_active_persona(&mut self, persona: Option<Persona>) {
        self.0.set_active_persona(persona);
    }
    fn set_personas(&mut self, personas: Vec<Persona>) {
        self.0.set_personas(personas);
    }
    fn active_persona(&self) -> Option<&Persona> {
        self.0.active_persona()
    }
}

impl GlobalToolDefinitionsWrite for ContextOps<'_> {
    fn global_tool_definitions_mut(&mut self) -> &mut BTreeMap<String, ToolDefinition> {
        &mut self.0.global_tool_definitions
    }
}

impl SessionToolDefinitionsWrite for ContextOps<'_> {
    fn session_tool_definitions_mut(
        &mut self,
    ) -> &mut BTreeMap<SessionId, BTreeMap<String, ToolDefinition>> {
        &mut self.0.session_tool_definitions
    }
}
// ── Projection method ────────────────────────────────────────────────────────

impl State {
    /// Write access to the context capsule, scoped via [`ContextView`].
    pub fn with_context<R, F>(&self, _cap: &ContextCap, f: F) -> R
    where
        F: FnOnce(&mut ContextView<'_>) -> R,
    {
        let mut guard = self.write_lock();
        let app = &mut *guard;
        f(&mut ContextView {
            context: ContextOps(&mut app.context),
        })
    }
}
