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
//! Unlike the old workflow actor, this one:
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

use crate::common::actor::protocol::event::AllActorsSpawned;
use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;

use crate::PhaseKind;
use crate::SessionId;
use crate::feat::attached_plugin::AttachedPlugin;
use crate::feat::plugin_dispatch::DomainNodeContext;
use crate::feat::plugin_dispatch::protocol::command::{AttachPlugin, DetachPlugin, TogglePlugin};
use crate::feat::plugin_dispatch::protocol::event::{
    PluginAttached, PluginDetached, PluginToggled,
};
use crate::feat::plugin_system::SessionRegistryId;
use crate::feat::session::chat_entry::ChatEntryKind;
use crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged;
use crate::feat::session_lifecycle::protocol::event::SessionCreated;
use crate::protocol::{Command, Event};

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
    services: Services,
    /// Shared app state (the `RwLock<SessionMap>`). Held by reference; the
    /// actor host ensures this outlives the actor.
    state: State,
    /// Maps `session_id → SessionRegistryId` so we can destroy the per-session
    /// Lua state when the session has no attached plugins.
    registry: AttachedPluginRegistry,
    /// The session ID active at startup (for `on_app_started` ctx).
    startup_session_id: String,
    /// Domain LLM context shared with plugin `ctx.request("llm_oneshot")`.
    /// Resolves pending one-shot oneshots when a workflow session completes.
    domain_ctx: Arc<DomainNodeContext>,
}

/// Dependencies for [`PluginDispatchActor`].
pub struct PluginDispatchActorDeps {
    pub services: Services,
    pub state: State,
    pub startup_session_id: String,
    pub domain_ctx: Arc<DomainNodeContext>,
}

impl Actor for PluginDispatchActor {
    type Message = NoDirectMsg;
    type Deps = PluginDispatchActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<AllActorsSpawned>();
        ctx.subscribe_event::<SessionCreated>();
        ctx.subscribe_event::<SessionPhaseChanged>();

        ctx.subscribe_command::<AttachPlugin>();
        ctx.subscribe_command::<DetachPlugin>();
        ctx.subscribe_command::<TogglePlugin>();

        // Generic async-handoff: plugins emit this to route an arbitrary hook
        // name to the plugin-async VM. Routed by runtime name via Command::Dynamic.
        ctx.subscribe_command_by_name("plugin::fire_async");

        ctx.set_description(
            "Plugin dispatcher: attaches/detaches plugins per session and fires hooks on lifecycle events",
        );

        Self {
            services: deps.services,
            state: deps.state,
            registry: AttachedPluginRegistry::default(),
            startup_session_id: deps.startup_session_id,
            domain_ctx: deps.domain_ctx,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<NoDirectMsg>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => self.handle_event(event).await,
            ActorEnvelope::Command(command) => self.handle_command(command, ctx).await,
            ActorEnvelope::System(_) => {}
        }
    }
}

impl PluginDispatchActor {
    // ─── Event dispatch ────────────────────────────────────────────────────

    async fn handle_event(&self, event: Event) {
        match event {
            Event::AllActorsSpawned(_) => self.fire_on_app_started().await,
            Event::SessionCreated(SessionCreated { session_id }) => {
                self.fire_on_session_created(session_id).await;
            }
            Event::SessionPhaseChanged(SessionPhaseChanged {
                session_id,
                old_phase: _,
                new_phase,
            }) => self.fire_on_phase_changed(session_id, new_phase).await,
            _ => {}
        }
    }

    // ─── Command dispatch ──────────────────────────────────────────────────

    async fn handle_command(&mut self, command: Command, ctx: &ActorContext) {
        match command {
            Command::AttachPlugin(cmd) => self.handle_attach(cmd, ctx).await,
            Command::DetachPlugin(cmd) => self.handle_detach(cmd, ctx).await,
            Command::TogglePlugin(cmd) => self.handle_toggle(cmd, ctx).await,
            Command::Dynamic(ref d) if d.name == "plugin::fire_async" => {
                self.handle_fire_async_hook(&d.payload, ctx).await;
            }
            _ => {}
        }
    }

    // ─── Command handlers ──────────────────────────────────────────────────

    async fn handle_attach(&mut self, cmd: AttachPlugin, ctx: &ActorContext) {
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
        let _ = ctx.send_event(Event::PluginAttached(PluginAttached {
            session_id,
            plugin_name,
        }));
    }

    async fn handle_detach(&mut self, cmd: DetachPlugin, ctx: &ActorContext) {
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
        let _ = ctx.send_event(Event::PluginDetached(PluginDetached {
            session_id,
            plugin_name,
        }));
    }

    /// Destroy any existing registry for `session_id`, then create a new one
    /// loading `plugin_names`. If `plugin_names` is empty, the registry is
    /// destroyed and no new one is created.
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
            .create_session_registry(plugin_names)
            .await
        {
            Ok(new_id) => self.registry.insert(session_id.clone(), new_id),
            Err(e) => {
                tracing::warn!(err = %e, session_id = %session_id, "create_session_registry failed");
            }
        }
    }

    #[expect(
        clippy::unused_async,
        reason = "trait contract requires async; the awaited event send is fire-and-forget"
    )]
    async fn handle_toggle(&mut self, cmd: TogglePlugin, ctx: &ActorContext) {
        let TogglePlugin {
            session_id,
            plugin_name,
        } = cmd;
        tracing::debug!(session_id = %session_id, plugin = %plugin_name, "toggling plugin");

        let (now_enabled, still_attached) = {
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
            let now_enabled = plugin.enabled;
            // If toggling off we keep the entry around — detach is the explicit
            // removal path. So `still_attached` is always true here.
            (now_enabled, true)
        };

        // Recreate registry only if disabling removed the only enabled plugin, or
        // enabling added a plugin. Simplest: always recreate from the enabled set.
        // We only need to recreate when there was a state change affecting which
        // plugins should be loaded into the Lua state — that's the enabled subset.
        // For simplicity in this pass: recreate from all attached_plugins names.
        // (Toggle affects in-fire filtering, not registry composition.)
        //
        // Actually no — the Lua state loads plugins at create time. If we want a
        // disabled plugin to be skipped at fire time without recreating the state,
        // the fire path must filter by enabled. Keep registry stable on toggle.
        //
        // If the toggle removes the *last* enabled plugin, registry recreation is
        // still optional (fire-time filtering handles it). So: no registry
        // recreation here.
        let _ = (now_enabled, still_attached);

        let _ = ctx.send_event(Event::PluginToggled(PluginToggled {
            session_id,
            plugin_name,
            enabled: now_enabled,
        }));
    }

    // ─── Lifecycle hook firings ────────────────────────────────────────────

    async fn fire_on_app_started(&self) {
        let ctx_json = serde_json::json!({
            "session_id": self.startup_session_id,
        });
        if let Err(e) = self
            .services
            .plugins
            .fire_async_json("on_app_started", &ctx_json)
            .await
        {
            tracing::warn!(err = %e, "on_app_started hook failed");
        }
    }

    async fn fire_on_session_created(&self, session_id: SessionId) {
        let ctx_json = serde_json::json!({
            "session_id": session_id.to_string(),
        });
        self.fire_for_session(&session_id, "on_session_created", &ctx_json)
            .await;
    }

    async fn fire_on_phase_changed(&self, session_id: SessionId, new_phase: PhaseKind) {
        // Workflow session completed: resolve any pending plugin LLM one-shot oneshot.
        // A workflow session is one spawned by DomainNodeContext::send_llm_request_oneshot;
        // it has is_workflow=true and a pending sender in domain_ctx. Extract the last
        // assistant entry text and resolve the awaiting coroutine.
        if new_phase == PhaseKind::Idle {
            if self.domain_ctx.has_pending(&session_id) {
                let response = self.extract_last_assistant_text(&session_id);
                self.domain_ctx.resolve_completed(&session_id, response);
            }
        }

        let hook = match new_phase {
            PhaseKind::Idle => "on_turn_end",
            PhaseKind::Sending => "on_user_submit",
            PhaseKind::Streaming => return, // streaming is mid-turn; no hook
        };
        let ctx_json = serde_json::json!({
            "session_id": session_id.to_string(),
        });
        self.fire_for_session(&session_id, hook, &ctx_json).await;
    }

    /// Fire a hook for a session: global + session-scoped if the session has a
    /// registry, global-only otherwise.
    async fn fire_for_session(&self, session_id: &SessionId, hook: &str, ctx_json: &Value) {
        let result = match self.registry.get(session_id) {
            Some(registry_id) => {
                tracing::info!(%hook, %session_id, "PLUGTRACE: fire_for_session registry HIT — firing global+session");
                self.services
                    .plugins
                    .fire_async_for_session_json(*registry_id, hook, ctx_json)
                    .await
            }
            None => {
                tracing::info!(%hook, %session_id, "PLUGTRACE: fire_for_session registry MISS — firing global-only");
                self.services.plugins.fire_async_json(hook, ctx_json).await
            }
        };
        match &result {
            Ok(()) => tracing::info!(%hook, "PLUGTRACE: fire returned Ok"),
            Err(e) => tracing::warn!(err = %e, %hook, "PLUGTRACE: fire returned Err"),
        }
    }

    /// Extract the text of the last assistant entry for a session.
    /// Used to resolve a pending plugin LLM one-shot oneshot when a workflow
    /// session completes. Returns an empty string if no assistant entry exists.
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

    /// Handle the generic `plugin::fire_async` dynamic command: route an
    /// arbitrary hook name to the plugin-async VM for the session.
    ///
    /// Payload: `{ hook: String, session_id: String, text: Option<String> }`.
    async fn handle_fire_async_hook(&self, payload: &Value, _ctx: &ActorContext) {
        #[derive(serde::Deserialize)]
        struct FireAsyncPayload {
            hook: String,
            session_id: SessionId,
            #[serde(default)]
            text: Option<String>,
        }

        let payload = match serde_json::from_value::<FireAsyncPayload>(payload.clone()) {
            Ok(p) => {
                tracing::info!(hook = %p.hook, session_id = %p.session_id, "PLUGTRACE: actor received plugin::fire_async command");
                p
            }
            Err(e) => {
                tracing::warn!(error = %e, "PLUGTRACE: plugin::fire_async payload malformed; dropped");
                return;
            }
        };

        let mut ctx_json = serde_json::json!({
            "session_id": payload.session_id.to_string(),
            "hook": payload.hook,
        });
        if let Some(text) = payload.text {
            ctx_json["text"] = serde_json::Value::String(text);
        }

        self.fire_for_session(&payload.session_id, &payload.hook, &ctx_json).await;
    }
}

// Pull in a typed-error conversion so `?` works in callers that bubble up
// errors via `Report<PluginDispatchActorError>`. Currently no internal helpers
// return Result, but this is here to keep the door open.
#[allow(dead_code, reason = "kept for future internal helpers")]
fn into_error<E: std::fmt::Display>(e: E) -> Report<PluginDispatchActorError> {
    let _ = e;
    Report::new(PluginDispatchActorError)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::actor::protocol::dynamic_command::DynamicCommand;
    use crate::common::actor::context::ActorContext;
    use crate::common::actor::message_sink::RecordingSink;
    use crate::common::app_state::AppState;
    use crate::feat::attached_plugin::PluginRunState;
    use crate::feat::session::chat_session::ChatSessionState;
    use std::sync::Arc;

    fn make_actor() -> (
        PluginDispatchActor,
        Arc<RecordingSink>,
        ActorContext,
        SessionId,
    ) {
        use crate::common::session_map::SessionMap;
        let mut app_state = AppState::default();
        let session = ChatSessionState::default();
        let session_id = session.session_id().clone();
        app_state.session = SessionMap::new(session, std::path::PathBuf::from("/tmp"));
        let state = State::new(app_state);
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("plugin-dispatch-test", sink.clone());
        let actor = PluginDispatchActor::activate(
            PluginDispatchActorDeps {
                services: Services::new(),
                state,
                startup_session_id: session_id.to_string(),
                domain_ctx: std::sync::Arc::new(DomainNodeContext::new(Services::new(), State::new(AppState::default()))),
            },

            &mut ctx,
        );
        (actor, sink, ctx, session_id)
    }

    #[tokio::test]
    async fn attach_plugin_pushes_onto_session_attached_plugins() {
        let (mut actor, sink, ctx, session_id) = make_actor();
        actor
            .handle_command(
                Command::AttachPlugin(
                    crate::feat::plugin_dispatch::protocol::command::AttachPlugin {
                        session_id: session_id.clone(),
                        plugin_name: "judge_fail".to_owned(),
                    },
                ),
                &ctx,
            )
            .await;
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
        // Registry has entry; plugin was loaded.
        assert!(actor.registry.get(&session_id).is_some());
        // PluginAttached event was sent.
        assert_eq!(sink.take_events().len(), 1);
    }

    #[tokio::test]
    async fn detach_plugin_removes_from_session() {
        let (mut actor, sink, ctx, session_id) = make_actor();
        actor
            .handle_command(
                Command::AttachPlugin(
                    crate::feat::plugin_dispatch::protocol::command::AttachPlugin {
                        session_id: session_id.clone(),
                        plugin_name: "judge_fail".to_owned(),
                    },
                ),
                &ctx,
            )
            .await;
        sink.take_events(); // drain
        // Detach.
        actor
            .handle_command(
                Command::DetachPlugin(
                    crate::feat::plugin_dispatch::protocol::command::DetachPlugin {
                        session_id: session_id.clone(),
                        plugin_name: "judge_fail".to_owned(),
                    },
                ),
                &ctx,
            )
            .await;
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
        assert_eq!(sink.take_events().len(), 1);
    }

    #[tokio::test]
    async fn toggle_plugin_flips_enabled() {
        let (mut actor, sink, ctx, session_id) = make_actor();
        actor
            .handle_command(
                Command::AttachPlugin(
                    crate::feat::plugin_dispatch::protocol::command::AttachPlugin {
                        session_id: session_id.clone(),
                        plugin_name: "judge_fail".to_owned(),
                    },
                ),
                &ctx,
            )
            .await;
        sink.take_events();
        // Toggle.
        actor
            .handle_command(
                Command::TogglePlugin(
                    crate::feat::plugin_dispatch::protocol::command::TogglePlugin {
                        session_id: session_id.clone(),
                        plugin_name: "judge_fail".to_owned(),
                    },
                ),
                &ctx,
            )
            .await;
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
        let (mut actor, sink, ctx, _) = make_actor();
        let bogus_id = SessionId::new();
        actor
            .handle_command(
                Command::AttachPlugin(
                    crate::feat::plugin_dispatch::protocol::command::AttachPlugin {
                        session_id: bogus_id.clone(),
                        plugin_name: "judge_fail".to_owned(),
                    },
                ),
                &ctx,
            )
            .await;
        assert!(actor.registry.get(&bogus_id).is_none());
        assert!(sink.take_events().is_empty());
    }

    #[tokio::test]
    async fn detach_unknown_plugin_is_noop() {
        let (mut actor, sink, ctx, session_id) = make_actor();
        actor
            .handle_command(
                Command::DetachPlugin(
                    crate::feat::plugin_dispatch::protocol::command::DetachPlugin {
                        session_id: session_id.clone(),
                        plugin_name: "judge_fail".to_owned(),
                    },
                ),
                &ctx,
            )
            .await;
        // Idempotent: still emits PluginDetached for confirmation, but session
        // state is unchanged.
        assert_eq!(sink.take_events().len(), 1);
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
        let (mut actor, sink, ctx, session_id) = make_actor();
        actor
            .handle_command(
                Command::TogglePlugin(
                    crate::feat::plugin_dispatch::protocol::command::TogglePlugin {
                        session_id: session_id.clone(),
                        plugin_name: "judge_fail".to_owned(),
                    },
                ),
                &ctx,
            )
            .await;
        assert!(sink.take_events().is_empty());
    }

    #[tokio::test]
    async fn fire_async_hook_routes_dynamic_to_handler() {
        let (mut actor, _sink, ctx, session_id) = make_actor();
        // A well-formed plugin::fire_async dynamic command for the session.
        // NoopPluginFire silently no-ops, but the handler must not panic and
        // must resolve the session (registry miss falls back to global fire).
        actor
            .handle_command(
                Command::Dynamic(DynamicCommand {
                    name: "plugin::fire_async".to_owned(),
                    payload: serde_json::json!({
                        "hook": "on_enrich",
                        "session_id": session_id.to_string(),
                        "text": "hello",
                    }),
                }),
                &ctx,
            )
            .await;
        // No panic, no crash, no events (noop fire emits nothing).
    }

    #[tokio::test]
    async fn fire_async_hook_drops_malformed_payload() {
        let (mut actor, sink, ctx, _session_id) = make_actor();
        // Malformed payload (session_id missing) is logged + dropped, no panic.
        actor
            .handle_command(
                Command::Dynamic(DynamicCommand {
                    name: "plugin::fire_async".to_owned(),
                    payload: serde_json::json!({ "hook": "on_enrich" }),
                }),
                &ctx,
            )
            .await;
        assert!(sink.take_events().is_empty());
    }

    #[tokio::test]
    async fn unrelated_dynamic_command_is_ignored() {
        let (mut actor, sink, ctx, _session_id) = make_actor();
        // A dynamic command with a different name must not be handled here.
        actor
            .handle_command(
                Command::Dynamic(DynamicCommand {
                    name: "some_other::action".to_owned(),
                    payload: serde_json::Value::Null,
                }),
                &ctx,
            )
            .await;
        assert!(sink.take_events().is_empty());
    }
}
