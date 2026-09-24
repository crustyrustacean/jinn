//! Test-only assembly service — the cross-compilation bridge for unit tests.
//!
//! `cargo test` compiles this crate twice: once as the `--cfg test` unit-test
//! binary and once as the plain rlib that downstream slices link against.
//! Trouper dispatches asks by lending the payload's live value, matched by
//! `TypeId` — so the real `context-assembly` service (compiled into the
//! plain rlib's copy of `AssembleContext`) can never recognize a request
//! built from the test binary's type. Under 0.8's erased-`Json` asks the
//! two copies were invisible; 0.9's live-value fabric exposes them.
//!
//! The stub closes the gap: it handles THIS compilation's `AssembleContext`
//! and delegates to the slice's real, pure `assemble`, so unit tests
//! exercise the genuine assembly pipeline over a same-typed ask. The
//! plain-rlib service keeps production behavior (single compilation there).

use error_stack::Report;
use trouper::actor::{ActorPath, MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;

use crate::feat::context::protocol::inputs::{AssembleContext, AssembledResponse};

/// The path the stub registers at — identical to the slice service's, so
/// test callers need no special addressing.
pub(crate) const CONTEXT_ASSEMBLY_TEST_PATH: &str = "context-assembly";

/// The stateless test stub: same contract as the slice's service, bound to
/// this compilation of the message types.
pub(crate) struct TestAssemblyService;

impl ServiceActor for TestAssemblyService {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "stateless service: start has nothing to await"
    )]
    async fn start(_args: &trouper::json::Json) -> Result<Self, Report<RegistryError>> {
        Ok(Self)
    }
}

impl MsgHandler<AssembleContext> for TestAssemblyService {
    async fn handle(&mut self, msg: &AssembleContext, ctx: &mut MsgCtx<'_>) {
        // The serde seam: every domain type in the ask is duplicated by
        // the two compilations, so the inputs cross in wire form — the
        // slice crate (singly compiled) deserializes into ITS types and
        // returns only wire-safe values.
        let inputs = serde_json::to_value(&msg.inputs).unwrap_or(serde_json::Value::Null);
        let prompt = jinn_context_assembly::assemble::assemble_erased(inputs).ok();
        let Some(prompt) = prompt else {
            tracing::error!("assembly test bridge: inputs failed to roundtrip");
            return;
        };
        ctx.reply(AssembledResponse {
            session_id: prompt.session_id.clone(),
            prompt,
        });
    }
}

/// Spawns the stub at the service's canonical path.
///
/// Idempotent at test scope via the same once-only path invariant the slice
/// service relies on: a duplicate registration panics, which the caller may
/// tolerate (`ensure` variants swallow it).
pub(crate) fn spawn(system: &trouper::system::ActorSystem) -> ActorPath {
    trouper::builder::spawn_service_builder::<TestAssemblyService>(system)
        .at(ActorPath::new(CONTEXT_ASSEMBLY_TEST_PATH))
        .handles::<AssembleContext>()
        .emits::<AssembledResponse>()
        .mailbox(64, trouper::inbox::OverloadPolicy::Block)
        .start()
}

/// Spawns the stub unless its path is already live.
pub(crate) fn ensure_spawned(system: &trouper::system::ActorSystem) -> Option<ActorPath> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| spawn(system)));
    result.ok()
}
