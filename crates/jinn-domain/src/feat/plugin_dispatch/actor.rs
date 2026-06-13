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
use crate::feat::attached_plugin::AttachedPlugin;
use crate::feat::plugin_dispatch::DomainNodeContext;
use crate::feat::plugin_dispatch::protocol::command::{
    AttachPlugin, DetachPlugin, SetManagedSession, TogglePlugin,
};
use crate::feat::plugin_dispatch::protocol::event::{
    PluginAttached, PluginDetached, PluginToggled,
};
use crate::feat::plugin_system::SessionRegistryId;
use crate::feat::session::chat_entry::ChatEntryKind;
use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
use crate::feat::session_lifecycle::protocol::event::SessionCreated;
use kameo::prelude::{Actor, ActorRef, Context, Message};
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
        use crate::common::services::bus_service::BusService;
        let bus = &args.deps.services.bus;

        bus.subscribe::<AllActorsSpawned, _>(&actor_ref).await;
        bus.subscribe::<SessionCreated, _>(&actor_ref).await;
        bus.subscribe::<SessionPhaseChanged, _>(&actor_ref).await;
        bus.subscribe::<AttachPlugin, _>(&actor_ref).await;
        bus.subscribe::<DetachPlugin, _>(&actor_ref).await;
        bus.subscribe::<TogglePlugin, _>(&actor_ref).await;
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
        let plugin_names: Vec<String> = {
            let state = &mut self.state.write().session;
            let Some(session) = state.get_mut(&session_id) else {
                tracing::warn!(session_id = %session_id, "session not found for attach");
                return;
            };
            session
                .core
                .attached_plugins
                .push(AttachedPlugin::new(&plugin_name));
            session
                .core
                .attached_plugins
                .iter()
                .map(|p| p.name.clone())
                .collect()
        };

        // 2. Destroy old registry (if any), create new with full plugin list.
        self.recreate_session_registry(&session_id, plugin_names)
            .await;

        // 3. Publish event.
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

        // 1. Remove AttachedPlugin from session.core.attached_plugins.
        let remaining_names: Vec<String> = {
            let state = &mut self.state.write().session;
            if let Some(session) = state.get_mut(&session_id) {
                session
                    .core
                    .attached_plugins
                    .retain(|p| p.name.as_str() != plugin_name.as_str());
                session
                    .core
                    .attached_plugins
                    .iter()
                    .map(|p| p.name.clone())
                    .collect()
            } else {
                tracing::warn!(session_id = %session_id, "session not found for detach");
                return;
            }
        };

        // 2. Destroy and recreate the registry (or destroy if no plugins remain).
        self.recreate_session_registry(&session_id, remaining_names)
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
        plugin_names: Vec<String>,
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

        // 2. Create new registry only if there are plugins to load.
        if plugin_names.is_empty() {
            return;
        }

        match self
            .services
            .session_plugin_registry
            .create_session_registry(plugin_names, session_id.clone())
            .await
        {
            Ok(result) => {
                self.registry.insert(session_id.clone(), result.registry_id);

                // 3. Register plugin tools with the tools actor.
                if !result.tool_metadata.is_empty() {
                    let registry_id = result.registry_id;
                    self.register_plugin_tools_with_actor(
                        session_id,
                        &registry_id,
                        result.tool_metadata,
                    )
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
        registry_id: &crate::feat::plugin_system::SessionRegistryId,
        tools: Vec<crate::feat::plugin_system::PluginToolMetadata>,
    ) {
        use crate::feat::plugin_system::ToolScope;
        use crate::feat::tools_actor::protocol::command::RegisterPluginTools;

        // Partition tools by scope: global vs attached.
        let mut global_by_plugin: std::collections::HashMap<
            String,
            Vec<crate::feat::plugin_system::PluginToolMetadata>,
        > = std::collections::HashMap::new();
        let mut attached_by_plugin: std::collections::HashMap<
            String,
            Vec<crate::feat::plugin_system::PluginToolMetadata>,
        > = std::collections::HashMap::new();

        for tool in tools {
            let map = match tool.scope {
                ToolScope::Global => &mut global_by_plugin,
                ToolScope::Attached => &mut attached_by_plugin,
            };
            map.entry(tool.plugin_name.clone()).or_default().push(tool);
        }

        // Register global tools (broadcast, no session target).
        for (plugin_name, plugin_tools) in global_by_plugin {
            let definitions: Vec<jinn_provider::ToolDefinition> = plugin_tools
                .into_iter()
                .map(|meta| meta.to_tool_definition())
                .collect();

            self.publish(RegisterPluginTools {
                plugin_name,
                target: None,
                session_id: None,
                definitions,
            })
            .await;
        }
        // Register attached tools (scoped to this session).
        for (plugin_name, plugin_tools) in attached_by_plugin {
            let definitions: Vec<jinn_provider::ToolDefinition> = plugin_tools
                .into_iter()
                .map(|meta| meta.to_tool_definition())
                .collect();

            self.publish(RegisterPluginTools {
                plugin_name,
                target: Some(*registry_id),
                session_id: Some(session_id.clone()),
                definitions,
            })
            .await;
        }
    }

    #[expect(
        clippy::unused_async,
        reason = "trait contract requires async; the awaited event send is fire-and-forget"
    )]
    async fn handle_toggle(&mut self, cmd: TogglePlugin) {
        let TogglePlugin {
            session_id,
            plugin_name,
        } = cmd;
        tracing::debug!(session_id = %session_id, plugin = %plugin_name, "toggling plugin");

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
                .find(|p| p.name.as_str() == plugin_name.as_str())
            else {
                tracing::warn!(session_id = %session_id, plugin = %plugin_name, "plugin not attached");
                return;
            };
            plugin.enabled = !plugin.enabled;
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

    fn handle_set_managed_session(&mut self, cmd: SetManagedSession) {
        let SetManagedSession {
            session_id,
            plugin_name,
            managed_session_id,
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
            .find(|p| p.name.as_str() == plugin_name.as_str())
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

        let enabled_plugins = {
            let state = self.state.read();
            state
                .session
                .get(session_id)
                .map(|s| {
                    s.core
                        .attached_plugins
                        .iter()
                        .filter(|p| p.enabled)
                        .map(|p| p.name.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };

        self.spawn_fire_for_session(session_id, hook, &ctx_json, enabled_plugins);
    }

    /// Fire a hook for a session on a background task, so the actor loop is not
    /// blocked while the hook runs.
    fn spawn_fire_for_session(
        &self,
        session_id: &SessionId,
        hook: &str,
        ctx_json: &Value,
        enabled_plugins: Vec<String>,
    ) {
        let plugins = self.services.plugins.clone();
        let registry_id = self.registry.get(session_id).copied();
        let hook = hook.to_owned();
        let ctx_json = ctx_json.clone();
        tokio::spawn(async move {
            let result = match registry_id {
                Some(rid) => {
                    plugins
                        .fire_async_for_session_json(rid, &hook, &ctx_json, enabled_plugins)
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
    use crate::feat::attached_plugin::PluginRunState;
    use crate::feat::plugin_dispatch::protocol::command::{
        AttachPlugin, DetachPlugin, TogglePlugin,
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
    async fn register_plugin_tools_global_has_no_target() {
        // Given a plugin dispatch actor.
        let (mut actor, audit, session_id) = make_actor().await;
        let registry_id = SessionRegistryId::new();

        // When registering global plugin tools.
        let tools = vec![test_tool_metadata("web_search", ToolScope::Global)];
        actor
            .register_plugin_tools_with_actor(&session_id, &registry_id, tools)
            .await;

        // Then a RegisterPluginTools message is published.
        let msgs: Vec<RegisterPluginTools> = audit.of_type::<RegisterPluginTools>();
        assert_eq!(msgs.len(), 1);
        // And the global tool has no target.
        assert!(msgs[0].target.is_none());
    }

    #[tokio::test]
    async fn register_plugin_tools_attached_has_target() {
        // Given a plugin dispatch actor.
        let (mut actor, audit, session_id) = make_actor().await;
        let registry_id = SessionRegistryId::new();

        // When registering attached plugin tools.
        let tools = vec![test_tool_metadata("judge", ToolScope::Attached)];
        actor
            .register_plugin_tools_with_actor(&session_id, &registry_id, tools)
            .await;

        // Then a RegisterPluginTools message is published.
        let msgs: Vec<RegisterPluginTools> = audit.of_type::<RegisterPluginTools>();
        assert_eq!(msgs.len(), 1);
        // And the attached tool targets this registry.
        assert_eq!(msgs[0].target, Some(registry_id));
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
    async fn toggle_plugin_flips_enabled() {
        // Given a plugin dispatch actor with a plugin attached.
        let (mut actor, _audit, session_id) = make_actor().await;
        actor
            .handle_attach(AttachPlugin {
                session_id: session_id.clone(),
                plugin_name: "judge_fail".to_owned(),
            })
            .await;

        // When toggling the plugin.
        actor
            .handle_toggle(TogglePlugin {
                session_id: session_id.clone(),
                plugin_name: "judge_fail".to_owned(),
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
    async fn toggle_unknown_plugin_is_noop() {
        // Given a plugin dispatch actor.
        let (mut actor, _audit, session_id) = make_actor().await;

        // When toggling a plugin that was never attached.
        actor
            .handle_toggle(TogglePlugin {
                session_id: session_id.clone(),
                plugin_name: "nonexistent".to_owned(),
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
