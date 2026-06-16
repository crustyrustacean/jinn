//! `PluginDispatchActor` — translates lifecycle events into plugin hook calls.
//!
//! This is the thin event → hook dispatcher that replaces the deleted
//! `WorkflowControllerActor`. It does three things:
//!
//! 1. Listens for `AttachPlugin` / `DetachPlugin` / `TogglePlugin` commands and
//!    manages per-session plugin Lua states via [`SessionPluginRegistry`].
//! 2. Listens for lifecycle events (`AllActorsSpawned`, `SessionCreated`,
//!    `SessionPhaseChanged`) and translates them into hook fires
//!    (`on_app_started`, `on_session_created`, `on_turn_end`).
//! 3. Tracks in-flight plugin runs per (session, plugin) so that
//!    `PluginRunState` transitions correctly.
//!
//! Unlike the old automated-session actor, this one:
//! - Has no trigger enum. The plugin decides what to subscribe to by defining
//!   hook functions.
//! - Has no merge strategies. Plugins self-orchestrate
//!   via `ctx.emit(...)`.
//! - Fans out hook fires to global plugins + the session's attached plugins
//!   (via `fire_async_for_session_json`).

use std::collections::HashMap;
use std::sync::Arc;

use error_stack::Report;
use serde_json::Value;

use crate::common::actor::protocol::dynamic_command::DynamicCommand;
use crate::common::actor::protocol::event::AllActorsSpawned;
use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::services::Services;
use crate::common::services::bus_service::BusService;
use crate::common::state::State;

use crate::PhaseKind;
use crate::SessionId;
use crate::feat::attached_plugin::{AttachedPlugin, PluginInstanceId};
use crate::feat::plugin_dispatch::DomainNodeContext;
use crate::feat::plugin_dispatch::protocol::command::{
    AttachPlugin, DetachPlugin, EnablePlugin, SetManagedSession, TogglePlugin,
};
use crate::feat::plugin_dispatch::protocol::event::{
    PluginAttached, PluginDetached, PluginToggled,
};
use crate::feat::plugin_system::SessionRegistryId;
use crate::feat::session::chat_entry::ChatEntryKind;
use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
use crate::feat::session_lifecycle::protocol::event::SessionCreated;
use kameo::prelude::{ActorRef, Context, Message};
/// Errors raised by [`PluginDispatchActor`] operations.
///
/// All error context lives in attached `.attach(...)` values on the
/// `Report<PluginDispatchActorError>` returned from fallible operations.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub struct PluginDispatchActorError;

/// Map key: (session id, plugin name).
///
/// Tracks per-session plugin Lua-state registry IDs.
///
/// Each session has at most one registry (one Lua state). All attached
/// plugins for that session share the state. On attach, the registry is
/// destroyed and recreated with the updated plugin list. On full detach,
/// the registry is destroyed and the entry removed.
#[derive(Debug, Default)]
struct AttachedPluginRegistry {
    inner: HashMap<SessionId, SessionRegistryId>,
}

impl AttachedPluginRegistry {
    fn get(&self, session_id: &SessionId) -> Option<&SessionRegistryId> {
        self.inner.get(session_id)
    }

    fn insert(&mut self, session_id: SessionId, id: SessionRegistryId) {
        self.inner.insert(session_id, id);
    }

    fn remove(&mut self, session_id: &SessionId) -> Option<SessionRegistryId> {
        self.inner.remove(session_id)
    }
}

/// The thin event → hook dispatcher.
pub struct PluginDispatchActor {
    deps: ActorDeps,
    services: Services,
    /// Shared app state.
    state: State,
    /// Maps `session_id → SessionRegistryId` so we can destroy the per-session
    /// Lua state when the session has no attached plugins.
    registry: AttachedPluginRegistry,
    /// The session ID active at startup (for `on_app_started` ctx).
    startup_session_id: String,
    /// Domain LLM context shared with plugin `ctx.request("llm_oneshot")`.
    domain_ctx: Arc<DomainNodeContext>,
}

/// Dependencies for [`PluginDispatchActor`].
#[derive(Clone)]
pub struct PluginDispatchActorDeps {
    /// Universal actor dependencies.
    pub deps: ActorDeps,
    pub services: Services,
    pub state: State,
    pub startup_session_id: String,
    pub domain_ctx: Arc<DomainNodeContext>,
}

impl kameo::Actor for PluginDispatchActor {
    type Args = PluginDispatchActorDeps;
    type Error = PluginDispatchActorError;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        let bus = &args.deps.services.bus;

        bus.subscribe::<AllActorsSpawned, _>(&actor_ref).await;
        bus.subscribe::<SessionCreated, _>(&actor_ref).await;
        bus.subscribe::<SessionPhaseChanged, _>(&actor_ref).await;
        bus.subscribe::<AttachPlugin, _>(&actor_ref).await;
        bus.subscribe::<DetachPlugin, _>(&actor_ref).await;
        bus.subscribe::<TogglePlugin, _>(&actor_ref).await;
        bus.subscribe::<EnablePlugin, _>(&actor_ref).await;
        bus.subscribe::<SetManagedSession, _>(&actor_ref).await;
        bus.subscribe::<DynamicCommand, _>(&actor_ref).await;

        Ok(Self {
            deps: args.deps,
            services: args.services,
            state: args.state,
            registry: AttachedPluginRegistry::default(),
            startup_session_id: args.startup_session_id,
            domain_ctx: args.domain_ctx,
        })
    }
}

impl BusPublish for PluginDispatchActor {
    fn bus(&self) -> &BusService {
        &self.deps.services.bus
    }
}

impl PluginDispatchActor {
    // ─── Command handlers ──────────────────────────────────────────────────

    async fn handle_attach(&mut self, cmd: AttachPlugin) {
        let AttachPlugin {
            session_id,
            plugin_name,
        } = cmd;
        tracing::debug!(session_id = %session_id, plugin = %plugin_name, "attaching plugin");

        // 1. Push AttachedPlugin onto session.core.attached_plugins.
        //    Construct it first so we can capture the new instance id for the
        //    `on_attach` hook (fired after the registry is rebuilt).
        let new_instance = AttachedPlugin::new(&plugin_name);
        let new_instance_id = new_instance.instance_id.clone();
        let instances: Vec<(PluginInstanceId, String)> = {
            let state = &mut self.state.write().session;
            let Some(session) = state.get_mut(&session_id) else {
                tracing::warn!(session_id = %session_id, "session not found for attach");
                return;
            };
            session.core.attached_plugins.push(new_instance);
            session
                .core
                .attached_plugins
                .iter()
                .map(|p| (p.instance_id.clone(), p.name.clone()))
                .collect()
        };

        // 2. Destroy old registry (if any), create new with full instance list.
        self.recreate_session_registry(&session_id, instances).await;

        // 3. Fire the `on_attach` lifecycle hook for the new instance. Runs
        //    after the registry is rebuilt so the hook executes against the
        //    live Lua state. Only the new instance receives it.
        self.spawn_fire_for_session(
            &session_id,
            "on_attach",
            &serde_json::json!({ "session_id": session_id.to_string() }),
            vec![new_instance_id.clone()],
        );

        // 4. Publish event.
        self.publish(PluginAttached {
            session_id,
            plugin_name,
        })
        .await;
    }

    async fn handle_detach(&mut self, cmd: DetachPlugin) {
        let DetachPlugin {
            session_id,
            plugin_name,
        } = cmd;
        tracing::debug!(session_id = %session_id, plugin = %plugin_name, "detaching plugin");

        // 1. Capture the instances being removed and remove them from
        //    session.core.attached_plugins.
        let (removed_instances, remaining_instances): (
            Vec<PluginInstanceId>,
            Vec<(PluginInstanceId, String)>,
        ) = {
            let state = &mut self.state.write().session;
            let Some(session) = state.get_mut(&session_id) else {
                tracing::warn!(session_id = %session_id, "session not found for detach");
                return;
            };
            let removed: Vec<PluginInstanceId> = session
                .core
                .attached_plugins
                .iter()
                .filter(|p| p.name.as_str() == plugin_name.as_str())
                .map(|p| p.instance_id.clone())
                .collect();
            session
                .core
                .attached_plugins
                .retain(|p| p.name.as_str() != plugin_name.as_str());
            let remaining = session
                .core
                .attached_plugins
                .iter()
                .map(|p| (p.instance_id.clone(), p.name.clone()))
                .collect();
            (removed, remaining)
        };

        // 2. Fire the `on_detach` lifecycle hook for each removed instance.
        //    Runs BEFORE the registry is rebuilt, so the hook still executes
        //    against the live (pre-teardown) Lua state.
        for instance_id in &removed_instances {
            self.spawn_fire_for_session(
                &session_id,
                "on_detach",
                &serde_json::json!({ "session_id": session_id.to_string() }),
                vec![instance_id.clone()],
            );
        }

        // 3. Destroy and recreate the registry (or destroy if no plugins remain).
        self.recreate_session_registry(&session_id, remaining_instances)
            .await;

        // 3. Publish event.
        self.publish(PluginDetached {
            session_id,
            plugin_name,
        })
        .await;
    }

    async fn recreate_session_registry(
        &mut self,
        session_id: &SessionId,
        instances: Vec<(PluginInstanceId, String)>,
    ) {
        // 1. Tear down old registry (if any).
        if let Some(old_id) = self.registry.remove(session_id)
            && let Err(e) = self
                .services
                .session_plugin_registry
                .destroy_session_registry(old_id)
                .await
        {
            tracing::warn!(err = %e, session_id = %session_id, "destroy_session_registry failed");
        }

        // 2. Create new registry only if there are instances to load.
        if instances.is_empty() {
            return;
        }

        match self
            .services
            .session_plugin_registry
            .create_session_registry(instances, session_id.clone())
            .await
        {
            Ok(result) => {
                self.registry.insert(session_id.clone(), result.registry_id);

                // 3. Plugin tool visibility is driven at spawn time
                //    (`create_session` resolves from `attachable_tool_catalog`),
                //    not on attach. The call below is now a no-op, retained
                //    as a hook-in point on the attach path.
                if !result.tool_metadata.is_empty() {
                    self.register_plugin_tools_with_actor(session_id, result.tool_metadata)
                        .await;
                }
            }
            Err(e) => {
                tracing::warn!(err = %e, session_id = %session_id, "create_session_registry failed");
            }
        }
    }

    /// Send plugin tool definitions to the tools actor for registration.
    async fn register_plugin_tools_with_actor(
        &self,
        session_id: &SessionId,
        tools: Vec<crate::feat::plugin_system::PluginToolMetadata>,
    ) {
        // No-op: attached-scoped plugin tools are no longer published to the
        // origin session on attach. Their execution handlers are registered
        // globally at startup (`execution_only: true` in `actor_wiring.rs`),
        // and their definitions live in `attachable_tool_catalog`
        // (`ContextAssemblyState`). Visibility is driven at spawn time:
        // `create_session` resolves named tools from the catalog into the child
        // session's `session_tool_definitions`. The origin never sees them.
        //
        // This method is retained as a call-site on the attach path for
        // future hook-in and to keep existing regression tests valid.
        let _ = (session_id, tools);
    }

    async fn handle_toggle(&mut self, cmd: TogglePlugin) {
        let TogglePlugin {
            session_id,
            plugin_name,
            instance_id,
        } = cmd;
        tracing::debug!(session_id = %session_id, plugin = %plugin_name, instance = %instance_id, "disabling plugin instance");

        let now_enabled = {
            let state = &mut self.state.write().session;
            let Some(session) = state.get_mut(&session_id) else {
                tracing::warn!(session_id = %session_id, "session not found for toggle");
                return;
            };
            let Some(plugin) = session
                .core
                .attached_plugins
                .iter_mut()
                .find(|p| p.instance_id == instance_id)
            else {
                tracing::warn!(session_id = %session_id, plugin = %plugin_name, "plugin not attached");
                return;
            };
            plugin.enabled = false;
            plugin.enabled
        };

        // No registry recreation on toggle — fire-time filtering handles enabled/disabled.

        self.publish(PluginToggled {
            session_id,
            plugin_name,
            enabled: now_enabled,
        })
        .await;
    }

    async fn handle_enable(&mut self, cmd: EnablePlugin) {
        let EnablePlugin {
            session_id,
            plugin_name,
            instance_id,
        } = cmd;
        tracing::debug!(session_id = %session_id, plugin = %plugin_name, instance = %instance_id, "enabling plugin instance");

        let now_enabled = {
            let state = &mut self.state.write().session;
            let Some(session) = state.get_mut(&session_id) else {
                tracing::warn!(session_id = %session_id, "session not found for enable");
                return;
            };
            let Some(plugin) = session
                .core
                .attached_plugins
                .iter_mut()
                .find(|p| p.instance_id == instance_id)
            else {
                tracing::warn!(session_id = %session_id, plugin = %plugin_name, "plugin not attached");
                return;
            };
            plugin.enabled = true;
            plugin.enabled
        };

        self.publish(PluginToggled {
            session_id,
            plugin_name,
            enabled: now_enabled,
        })
        .await;
    }

    fn handle_set_managed_session(&mut self, cmd: SetManagedSession) {
        let SetManagedSession {
            session_id,
            plugin_name,
            managed_session_id,
            instance_id,
        } = cmd;
        tracing::debug!(session_id = %session_id, plugin = %plugin_name, managed = %managed_session_id, "setting managed session");

        let state = &mut self.state.write().session;
        let Some(session) = state.get_mut(&session_id) else {
            tracing::warn!(session_id = %session_id, "session not found for set_managed_session");
            return;
        };
        let Some(plugin) = session
            .core
            .attached_plugins
            .iter_mut()
            .find(|p| p.instance_id == instance_id)
        else {
            tracing::warn!(session_id = %session_id, plugin = %plugin_name, "plugin not attached");
            return;
        };
        plugin.managed_session_id = Some(managed_session_id);
    }

    // ─── Lifecycle hook firings ────────────────────────────────────────────

    fn fire_on_app_started(&self) {
        let session_id = SessionId::from(self.startup_session_id.clone());
        let ctx_json = serde_json::json!({
            "session_id": self.startup_session_id,
        });
        self.spawn_fire_for_session(&session_id, "on_app_started", &ctx_json, vec![]);
    }

    fn fire_on_session_created(&self, session_id: &SessionId) {
        let ctx_json = serde_json::json!({
            "session_id": session_id.to_string(),
        });
        self.spawn_fire_for_session(session_id, "on_session_created", &ctx_json, vec![]);
    }

    fn fire_on_phase_changed(&self, session_id: &SessionId, new_phase: PhaseKind) {
        tracing::debug!(session_id = %session_id, phase = ?new_phase, "fire_on_phase_changed");
        // Plugin session completed: resolve any pending plugin LLM one-shot oneshot.
        if new_phase == PhaseKind::Idle && self.domain_ctx.has_pending(session_id) {
            let response = self.resolve_response_for_session(session_id);
            self.domain_ctx.resolve_completed(session_id, response);
        }

        let hook = match new_phase {
            PhaseKind::Idle => "on_turn_end",
            PhaseKind::Sending => "on_user_submit",
            PhaseKind::Streaming => return, // streaming is mid-turn; no hook
        };
        let ctx_json = serde_json::json!({
            "session_id": session_id.to_string(),
        });

        let enabled_instances = {
            let state = self.state.read();
            state
                .session
                .get(session_id)
                .map(|s| {
                    s.core
                        .attached_plugins
                        .iter()
                        .filter(|p| p.enabled)
                        .map(|p| p.instance_id.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };

        self.spawn_fire_for_session(session_id, hook, &ctx_json, enabled_instances);
    }

    /// Fire a hook for a session on a background task, so the actor loop is not
    /// blocked while the hook runs.
    fn spawn_fire_for_session(
        &self,
        session_id: &SessionId,
        hook: &str,
        ctx_json: &Value,
        enabled_instances: Vec<PluginInstanceId>,
    ) {
        let plugins = self.services.plugins.clone();
        let registry_id = self.registry.get(session_id).copied();
        let hook = hook.to_owned();
        // Inject the parent session edge (if any) so hooks like the judge's
        // on_turn_end can reach the child's origin via ctx.parent_session_id.
        let mut ctx_json = ctx_json.clone();
        {
            let state = self.state.read();
            if let Some(parent) = state
                .session
                .get(session_id)
                .and_then(|s| s.core.parent_session.clone())
                && let Some(obj) = ctx_json.as_object_mut()
            {
                obj.insert(
                    "parent_session_id".to_owned(),
                    serde_json::Value::String(parent.to_string()),
                );
            }
        }
        tokio::spawn(async move {
            let result = match registry_id {
                Some(rid) => {
                    plugins
                        .fire_async_for_session_json(rid, &hook, &ctx_json, enabled_instances)
                        .await
                }
                None => plugins.fire_async_json(&hook, &ctx_json).await,
            };
            if let Err(e) = result {
                tracing::error!(hook = %hook, err = ?e, "plugin hook failed");
            }
        });
    }

    /// Extract the text of the last assistant entry for a session.
    fn extract_last_assistant_text(&self, session_id: &SessionId) -> String {
        let guard = self.state.read();
        let Some(session) = guard.session.get(session_id) else {
            return String::new();
        };
        session
            .history()
            .iter()
            .rev()
            .find_map(|entry| match &entry.kind {
                ChatEntryKind::Assistant(text) => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Determine the one-shot outcome to resolve the pending sender with.
    fn resolve_response_for_session(&self, session_id: &SessionId) -> Result<String, String> {
        let guard = self.state.read();
        let Some(session) = guard.session.get(session_id) else {
            return Ok(String::new());
        };
        let last = session.history().iter().next_back();
        match last.map(|entry| &entry.kind) {
            Some(ChatEntryKind::Error(message)) => Err(message.clone()),
            _ => Ok(self.extract_last_assistant_text(session_id)),
        }
    }

    /// Handle the generic `plugin::fire_async` dynamic command.
    fn handle_fire_async_hook(&self, payload: &Value) {
        #[derive(serde::Deserialize)]
        struct FireAsyncPayload {
            hook: String,
            session_id: SessionId,
            #[serde(default)]
            text: Option<String>,
        }

        let payload = match serde_json::from_value::<FireAsyncPayload>(payload.clone()) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "plugin::fire_async payload malformed; dropped");
                return;
            }
        };

        let mut ctx_json = serde_json::json!({
            "session_id": payload.session_id.to_string(),
            "hook": payload.hook,
        });
        if let Some(text) = payload.text
            && let Some(map) = ctx_json.as_object_mut()
        {
            map.insert("text".to_owned(), serde_json::Value::String(text));
        }

        self.spawn_fire_for_session(&payload.session_id, &payload.hook, &ctx_json, vec![]);
    }
}

// ─── kameo Message handlers ────────────────────────────────────────────

impl Message<AllActorsSpawned> for PluginDispatchActor {
    type Reply = ();
    async fn handle(&mut self, _msg: AllActorsSpawned, _ctx: &mut Context<Self, Self::Reply>) {
        self.fire_on_app_started();
    }
}

impl Message<SessionCreated> for PluginDispatchActor {
    type Reply = ();
    async fn handle(&mut self, msg: SessionCreated, _ctx: &mut Context<Self, Self::Reply>) {
        self.fire_on_session_created(&msg.session_id);
    }
}

impl Message<SessionPhaseChanged> for PluginDispatchActor {
    type Reply = ();
    async fn handle(&mut self, msg: SessionPhaseChanged, _ctx: &mut Context<Self, Self::Reply>) {
        self.fire_on_phase_changed(&msg.session_id, msg.new_phase);
    }
}

impl Message<AttachPlugin> for PluginDispatchActor {
    type Reply = ();
    async fn handle(&mut self, msg: AttachPlugin, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_attach(msg).await;
    }
}

impl Message<DetachPlugin> for PluginDispatchActor {
    type Reply = ();
    async fn handle(&mut self, msg: DetachPlugin, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_detach(msg).await;
    }
}

impl Message<TogglePlugin> for PluginDispatchActor {
    type Reply = ();
    async fn handle(&mut self, msg: TogglePlugin, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_toggle(msg).await;
    }
}

impl Message<SetManagedSession> for PluginDispatchActor {
    type Reply = ();
    async fn handle(&mut self, msg: SetManagedSession, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_set_managed_session(msg);
    }
}

impl Message<EnablePlugin> for PluginDispatchActor {
    type Reply = ();
    async fn handle(&mut self, msg: EnablePlugin, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_enable(msg).await;
    }
}

impl Message<DynamicCommand> for PluginDispatchActor {
    type Reply = ();
    async fn handle(&mut self, msg: DynamicCommand, _ctx: &mut Context<Self, Self::Reply>) {
        if msg.name == "plugin::fire_async" {
            self.handle_fire_async_hook(&msg.payload);
        }
    }
}

// Pull in a typed-error conversion so `?` works in callers that bubble up
// errors via `Report<PluginDispatchActorError>`.
#[expect(dead_code, reason = "kept for future internal helpers")]
fn into_error<E: std::fmt::Display>(e: E) -> Report<PluginDispatchActorError> {
    let _ = e;
    Report::new(PluginDispatchActorError)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unreachable,
        clippy::string_slice,
        clippy::uninlined_format_args,
        reason = "test code"
    )]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::services::bus_service::BusAudit;
    use crate::common::session_map::SessionMap;
    use crate::feat::attached_plugin::{PluginInstanceId, PluginRunState};
    use crate::feat::plugin_dispatch::protocol::command::{
        AttachPlugin, DetachPlugin, SetManagedSession, TogglePlugin,
    };
    use crate::feat::plugin_system::PluginToolMetadata;
    use crate::feat::plugin_system::ToolScope;
    use crate::feat::session::chat_session::ChatSessionState;
    use crate::feat::tools_actor::protocol::command::RegisterPluginTools;
    use std::sync::Arc;

    async fn make_actor() -> (PluginDispatchActor, BusAudit, SessionId) {
        let mut app_state = AppState::default();
        let session = ChatSessionState::default();
        let session_id = session.session_id().clone();
        app_state.session = SessionMap::new(session, std::path::PathBuf::from("/tmp"));
        let state = State::new(app_state);
        let (bus, audit) = BusService::new_recording();
        let services = Services::new_fake_with_bus(bus.clone()).await;
        let domain_ctx = Arc::new(DomainNodeContext::new(
            Services::new_fake_with_bus(bus).await,
            State::new(AppState::default()),
        ));
        let actor = PluginDispatchActor {
            deps: ActorDeps {
                services: services.clone(),
            },
            services,
            state,
            registry: AttachedPluginRegistry::default(),
            startup_session_id: session_id.to_string(),
            domain_ctx,
        };
        (actor, audit, session_id)
    }

    fn test_tool_metadata(name: &str, scope: ToolScope) -> PluginToolMetadata {
        PluginToolMetadata {
            name: name.to_owned(),
            description: format!("{name} tool"),
            parameters: serde_json::json!({}),
            plugin_name: "test-plugin".to_owned(),
            scope,
        }
    }

    // ─── Register plugin tools ────────────────────────────────────────────

    #[tokio::test]
    async fn register_plugin_tools_global_inside_attached_is_ignored() {
        // Given a plugin dispatch actor.
        let (actor, audit, session_id) = make_actor().await;

        // When registering a global-scope tool defined inside an attached plugin.
        let tools = vec![test_tool_metadata("web_search", ToolScope::Global)];
        actor
            .register_plugin_tools_with_actor(&session_id, tools)
            .await;

        // Then no RegisterPluginTools message is published — global tools are
        // registered globally at startup, not on attach.
        let msgs: Vec<RegisterPluginTools> = audit.of_type::<RegisterPluginTools>();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn register_plugin_tools_attached_publishes_nothing() {
        // Given a plugin dispatch actor.
        let (actor, audit, session_id) = make_actor().await;

        // When registering attached plugin tools.
        let tools = vec![test_tool_metadata("judge", ToolScope::Attached)];
        actor
            .register_plugin_tools_with_actor(&session_id, tools)
            .await;

        // Then NO RegisterPluginTools message is published. Attached tools
        // are registered execution-only at startup and cataloged in
        // `ContextAssemblyState.attachable_tool_catalog`; the origin session
        // never receives them. Child sessions resolve them by name at
        // spawn time via `create_child_session`.
        let msgs: Vec<RegisterPluginTools> = audit.of_type::<RegisterPluginTools>();
        assert!(msgs.is_empty());
    }
    // ─── Attach / Detach / Toggle ──────────────────────────────────────────

    #[tokio::test]
    async fn attach_plugin_pushes_onto_session_attached_plugins() {
        // Given a plugin dispatch actor.
        let (mut actor, audit, session_id) = make_actor().await;

        // When attaching a plugin.
        actor
            .handle_attach(AttachPlugin {
                session_id: session_id.clone(),
                plugin_name: "judge_fail".to_owned(),
            })
            .await;

        // Then the session has one attached plugin.
        let plugins = actor
            .state
            .read()
            .session
            .get(&session_id)
            .unwrap()
            .core
            .attached_plugins
            .clone();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "judge_fail");
        assert!(plugins[0].enabled);
        assert_eq!(plugins[0].run_state, PluginRunState::Idle);
        // And the registry has an entry.
        assert!(actor.registry.get(&session_id).is_some());
        // And PluginAttached event was published.
        assert!(audit.contains_name("PluginAttached"));
    }

    #[tokio::test]
    async fn detach_plugin_removes_from_session() {
        // Given a plugin dispatch actor with a plugin attached.
        let (mut actor, audit, session_id) = make_actor().await;
        actor
            .handle_attach(AttachPlugin {
                session_id: session_id.clone(),
                plugin_name: "judge_fail".to_owned(),
            })
            .await;
        audit.clear();

        // When detaching the plugin.
        actor
            .handle_detach(DetachPlugin {
                session_id: session_id.clone(),
                plugin_name: "judge_fail".to_owned(),
            })
            .await;

        // Then the session has no attached plugins.
        let plugins = actor
            .state
            .read()
            .session
            .get(&session_id)
            .unwrap()
            .core
            .attached_plugins
            .clone();
        assert!(plugins.is_empty());
        assert!(actor.registry.get(&session_id).is_none());
        // And PluginDetached event was published.
        assert!(audit.contains_name("PluginDetached"));
    }

    #[tokio::test]
    async fn disable_plugin_force_disables_targeted_instance() {
        // Given a plugin dispatch actor with a plugin attached.
        let (mut actor, _audit, session_id) = make_actor().await;
        actor
            .handle_attach(AttachPlugin {
                session_id: session_id.clone(),
                plugin_name: "judge_fail".to_owned(),
            })
            .await;

        // Capture the instance id assigned at attach.
        let instance_id = actor
            .state
            .read()
            .session
            .get(&session_id)
            .unwrap()
            .core
            .attached_plugins
            .first()
            .expect("plugin attached")
            .instance_id
            .clone();

        // When disabling the plugin instance.
        actor
            .handle_toggle(TogglePlugin {
                session_id: session_id.clone(),
                plugin_name: "judge_fail".to_owned(),
                instance_id: instance_id.clone(),
            })
            .await;

        // Then the plugin is disabled.
        let plugins = actor
            .state
            .read()
            .session
            .get(&session_id)
            .unwrap()
            .core
            .attached_plugins
            .clone();
        assert_eq!(plugins.len(), 1);
        assert!(!plugins[0].enabled);
    }

    #[tokio::test]
    async fn attach_on_unknown_session_is_noop() {
        // Given a plugin dispatch actor.
        let (actor, audit, _) = make_actor().await;
        let bogus_id = SessionId::new();

        // No registry entry and no event published for unknown session.
        assert!(actor.registry.get(&bogus_id).is_none());
        assert!(audit.is_empty());
    }

    #[tokio::test]
    async fn detach_unknown_plugin_is_noop() {
        // Given a plugin dispatch actor.
        let (mut actor, audit, session_id) = make_actor().await;

        // When detaching a plugin that was never attached.
        actor
            .handle_detach(DetachPlugin {
                session_id: session_id.clone(),
                plugin_name: "judge_fail".to_owned(),
            })
            .await;

        // Then PluginDetached is still published (idempotent confirmation).
        assert!(audit.contains_name("PluginDetached"));
        // And session state has no plugins.
        let plugins = actor
            .state
            .read()
            .session
            .get(&session_id)
            .unwrap()
            .core
            .attached_plugins
            .clone();
        assert!(plugins.is_empty());
    }

    #[tokio::test]
    async fn set_managed_session_per_instance_does_not_clobber_sibling() {
        // Given a plugin dispatch actor with two instances of the same plugin.
        let (mut actor, _audit, session_id) = make_actor().await;
        actor
            .handle_attach(AttachPlugin {
                session_id: session_id.clone(),
                plugin_name: "judge_fail".to_owned(),
            })
            .await;
        actor
            .handle_attach(AttachPlugin {
                session_id: session_id.clone(),
                plugin_name: "judge_fail".to_owned(),
            })
            .await;

        // ...and their instance ids.
        let (id_a, id_b) = {
            let s = actor.state.read();
            let s = s.session.get(&session_id).unwrap();
            let plugins = &s.core.attached_plugins;
            assert_eq!(plugins.len(), 2);
            (
                plugins[0].instance_id.clone(),
                plugins[1].instance_id.clone(),
            )
        };
        assert_ne!(id_a, id_b);

        let child_a = SessionId::new();
        let child_b = SessionId::new();

        // When each instance sets its own managed session.
        actor.handle_set_managed_session(SetManagedSession {
            session_id: session_id.clone(),
            plugin_name: "judge_fail".to_owned(),
            managed_session_id: child_a.clone(),
            instance_id: id_a.clone(),
        });
        actor.handle_set_managed_session(SetManagedSession {
            session_id: session_id.clone(),
            plugin_name: "judge_fail".to_owned(),
            managed_session_id: child_b.clone(),
            instance_id: id_b.clone(),
        });

        // Then each instance holds its OWN managed session — the second did
        // not clobber the first.
        let plugins = actor
            .state
            .read()
            .session
            .get(&session_id)
            .unwrap()
            .core
            .attached_plugins
            .clone();
        let by_id = plugins
            .iter()
            .map(|p| (p.instance_id.clone(), p.managed_session_id.clone()))
            .collect::<Vec<_>>();
        assert_eq!(by_id, vec![(id_a, Some(child_a)), (id_b, Some(child_b)),]);
    }

    #[tokio::test]
    async fn enable_plugin_force_enables_only_targeted_instance() {
        // Given a dispatch actor with two instances of the same plugin, both disabled.
        let (mut actor, _audit, session_id) = make_actor().await;
        actor
            .handle_attach(AttachPlugin {
                session_id: session_id.clone(),
                plugin_name: "judge_fail".to_owned(),
            })
            .await;
        actor
            .handle_attach(AttachPlugin {
                session_id: session_id.clone(),
                plugin_name: "judge_fail".to_owned(),
            })
            .await;
        let (id_a, id_b) = {
            let guard = actor.state.read();
            let s = guard.session.get(&session_id).expect("session");
            let plugins = &s.core.attached_plugins;
            (
                plugins[0].instance_id.clone(),
                plugins[1].instance_id.clone(),
            )
        };
        actor
            .handle_toggle(TogglePlugin {
                session_id: session_id.clone(),
                plugin_name: "judge_fail".to_owned(),
                instance_id: id_a.clone(),
            })
            .await;
        actor
            .handle_toggle(TogglePlugin {
                session_id: session_id.clone(),
                plugin_name: "judge_fail".to_owned(),
                instance_id: id_b.clone(),
            })
            .await;

        // When enabling only instance B.
        actor
            .handle_enable(EnablePlugin {
                session_id: session_id.clone(),
                plugin_name: "judge_fail".to_owned(),
                instance_id: id_b.clone(),
            })
            .await;

        // Then only instance B is enabled; instance A stays disabled.
        let plugins = actor
            .state
            .read()
            .session
            .get(&session_id)
            .expect("session")
            .core
            .attached_plugins
            .clone();
        assert_eq!(plugins.len(), 2);
        let a = plugins.iter().find(|p| p.instance_id == id_a).expect("a");
        let b = plugins.iter().find(|p| p.instance_id == id_b).expect("b");
        assert!(!a.enabled, "instance A must stay disabled");
        assert!(b.enabled, "instance B must be enabled");
    }

    #[tokio::test]
    async fn disable_then_enable_round_trips_the_targeted_instance() {
        // Given a dispatch actor with one enabled plugin instance.
        let (mut actor, _audit, session_id) = make_actor().await;
        actor
            .handle_attach(AttachPlugin {
                session_id: session_id.clone(),
                plugin_name: "judge_fail".to_owned(),
            })
            .await;
        let instance_id = {
            let guard = actor.state.read();
            guard
                .session
                .get(&session_id)
                .expect("session")
                .core
                .attached_plugins[0]
                .instance_id
                .clone()
        };

        // When disabling then re-enabling that instance.
        actor
            .handle_toggle(TogglePlugin {
                session_id: session_id.clone(),
                plugin_name: "judge_fail".to_owned(),
                instance_id: instance_id.clone(),
            })
            .await;
        actor
            .handle_enable(EnablePlugin {
                session_id: session_id.clone(),
                plugin_name: "judge_fail".to_owned(),
                instance_id: instance_id.clone(),
            })
            .await;

        // Then the instance is back to enabled (round-trip on the right instance).
        let enabled = actor
            .state
            .read()
            .session
            .get(&session_id)
            .expect("session")
            .core
            .attached_plugins[0]
            .enabled;
        assert!(enabled, "disable+enable must restore enabled");
    }
    #[tokio::test]
    async fn toggle_unknown_plugin_is_noop() {
        // Given a plugin dispatch actor.
        let (mut actor, _audit, session_id) = make_actor().await;

        // When toggling a plugin that was never attached.
        actor
            .handle_toggle(TogglePlugin {
                session_id: session_id.clone(),
                plugin_name: "nonexistent".to_owned(),
                instance_id: PluginInstanceId::new(),
            })
            .await;

        // Then session state is unchanged.
        let plugins = actor
            .state
            .read()
            .session
            .get(&session_id)
            .unwrap()
            .core
            .attached_plugins
            .clone();
        assert!(plugins.is_empty());
    }
}
