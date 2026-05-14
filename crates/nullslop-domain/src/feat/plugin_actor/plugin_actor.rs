//! Plugin actor — orchestrates plugin loading and event dispatch.
//!
//! The plugin actor manages a dedicated OS thread for rhai execution,
//! loads plugins from disk, forwards events, and handles plugin lifecycle.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use nullslop_plugin::runtime::{HostCallbacks, PluginSlotInfo};
use nullslop_plugin::{PluginId, PluginRuntime};

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::state::State;
use crate::feat::plugin_actor::protocol::command::ReloadScripts;
use crate::protocol::{ChatEntry, Command, Event};

/// Messages sent from the async actor to the plugin thread.
enum PluginThreadMsg {
    /// Forward an event to all plugins.
    ForwardEvent(String, Event),
    /// Reload all plugins from disk.
    ReloadAll,
    /// Shut down the plugin thread.
    Shutdown,
}

/// The plugin actor — bridges the actor system with rhai plugin scripts.
///
/// Spawns a dedicated OS thread for rhai execution (rhai's `Engine` is not
/// `Send`). The async actor forwards events and commands to the plugin thread
/// via an `mpsc` channel.
pub struct PluginActor {
    /// Sender to the plugin thread.
    tx: Option<std::sync::mpsc::Sender<PluginThreadMsg>>,
    /// Handle to the plugin thread.
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl Actor for PluginActor {
    type Message = NoDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        // Subscribe to ALL events (wildcard).
        ctx.subscribe_all_events();
        ctx.subscribe_command::<ReloadScripts>();
        ctx.set_description("Loads and manages rhai plugins");

        let state = ctx
            .take_data::<State>()
            .expect("PluginActor requires State injection");
        let plugins_dir = ctx
            .take_data::<PathBuf>()
            .unwrap_or_else(nullslop_plugin::loader::plugins_dir);
        let sink = ctx.sink();

        let (tx, rx) = std::sync::mpsc::channel::<PluginThreadMsg>();
        let handle = std::thread::spawn(move || {
            plugin_thread_main(rx, state, sink, plugins_dir);
        });

        Self {
            tx: Some(tx),
            thread_handle: Some(handle),
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<NoDirectMsg>, _ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => {
                self.forward_event(&event);
            }
            ActorEnvelope::Command(cmd) => {
                self.handle_command(&cmd);
            }
            ActorEnvelope::System(_) => {}
            ActorEnvelope::Direct(_) => {}
        }
    }

    async fn shutdown(mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(PluginThreadMsg::Shutdown);
        }
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

impl PluginActor {
    /// Forwards an event to the plugin thread.
    fn forward_event(&self, event: &Event) {
        let Some(tx) = &self.tx else { return };
        let Some(type_name) = event.type_name() else {
            return;
        };
        let _ = tx.send(PluginThreadMsg::ForwardEvent(
            type_name.to_owned(),
            event.clone(),
        ));
    }

    /// Handles a command.
    fn handle_command(&self, cmd: &Command) {
        let Some(tx) = &self.tx else { return };
        match cmd {
            Command::ReloadScripts(..) => {
                let _ = tx.send(PluginThreadMsg::ReloadAll);
            }
            _ => {}
        }
    }
}

/// Main loop for the dedicated plugin thread.
///
/// Manages a `HashMap<PluginId, PluginRuntime>`, loads plugins from disk,
/// and forwards events to plugins that have subscribed to them.
fn plugin_thread_main(
    rx: std::sync::mpsc::Receiver<PluginThreadMsg>,
    state: State,
    sink: Arc<dyn crate::common::actor::MessageSink>,
    plugins_dir: PathBuf,
) {
    let mut runtimes: HashMap<PluginId, PluginRuntime> = HashMap::new();
    let event_subscriptions: Arc<Mutex<HashMap<String, Vec<PluginId>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let callbacks = build_host_callbacks(&state, &sink, &event_subscriptions);

    // Initial load.
    load_all_plugins(&plugins_dir, &callbacks, &mut runtimes);

    while let Ok(msg) = rx.recv() {
        match msg {
            PluginThreadMsg::ForwardEvent(type_name, event) => {
                let subs = event_subscriptions.lock().unwrap();
                let Some(subscribed_ids) = subs.get(&type_name) else {
                    continue;
                };
                for plugin_id in subscribed_ids {
                    if let Some(runtime) = runtimes.get_mut(plugin_id) {
                        let event_map = event_to_rhai_map(&type_name, &event);
                        tracing::debug!(plugin = %plugin_id, event_type = %type_name, "forwarding event to plugin");
                        if let Err(e) = runtime.call_on_event(event_map) {
                            tracing::error!(
                                plugin = %plugin_id,
                                err = ?e,
                                "plugin on_event failed, disabling"
                            );
                            runtime.disable();
                            clear_slots_for_plugin(&state, plugin_id);
                            push_system_entry(
                                &state,
                                &format!("Plugin '{plugin_id}' disabled due to error"),
                            );
                        }
                    }
                }
            }
            PluginThreadMsg::ReloadAll => {
                runtimes.clear();
                state.write().plugin_slots.clear();
                load_all_plugins(&plugins_dir, &callbacks, &mut runtimes);
            }
            PluginThreadMsg::Shutdown => {
                break;
            }
        }
    }
}

/// Discovers and loads all plugins from the plugins directory.
fn load_all_plugins(
    plugins_dir: &PathBuf,
    callbacks: &Arc<HostCallbacks>,
    runtimes: &mut HashMap<PluginId, PluginRuntime>,
) {
    let discovered = nullslop_plugin::loader::discover(plugins_dir);
    tracing::info!(count = discovered.len(), dir = %plugins_dir.display(), "discovered plugins");
    for plugin in discovered {
        match PluginRuntime::load(plugin.id.clone(), &plugin.path, callbacks.clone()) {
            Ok(mut runtime) => {
                tracing::info!(plugin = %plugin.id, "loaded plugin, calling init");
                if let Err(e) = runtime.call_init() {
                    tracing::error!(plugin = %plugin.id, err = ?e, "plugin init failed");
                    continue;
                }
                tracing::info!(plugin = %plugin.id, "plugin initialized successfully");
                runtimes.insert(plugin.id.clone(), runtime);
            }
            Err(e) => {
                tracing::error!(plugin = %plugin.id, err = ?e, "plugin load failed");
            }
        }
    }
}

/// Builds the host callbacks that bridge rhai to Rust state.
fn build_host_callbacks(
    state: &State,
    sink: &Arc<dyn crate::common::actor::MessageSink>,
    event_subscriptions: &Arc<Mutex<HashMap<String, Vec<PluginId>>>>,
) -> Arc<HostCallbacks> {
    let state_clone = state.clone();
    let sink_clone = sink.clone();
    let subs = event_subscriptions.clone();

    Arc::new(HostCallbacks {
        subscribe_events: Arc::new(move |plugin_id: &PluginId, types: &[String]| {
            let mut subs = subs.lock().unwrap();
            for type_name in types {
                subs.entry(type_name.clone())
                    .or_default()
                    .push(plugin_id.clone());
            }
        }),
        emit_event: Arc::new(|_name: &str, _value: serde_json::Value| {
            // Future: emit inter-plugin events
        }),
        upsert_slot: Arc::new(move |info: PluginSlotInfo| {
            let section = match info.section.as_str() {
                "right" => crate::feat::ui::status_bar::SlotSection::Right,
                _ => crate::feat::ui::status_bar::SlotSection::Left,
            };
            let slot = crate::feat::ui::status_bar::PluginSlot {
                plugin_id: info.plugin_id.clone(),
                slot_id: uuid::Uuid::new_v4(),
                stable_id: info.stable_id,
                section,
                priority: info.priority as u32,
                text: info.text,
            };
            state_clone.write().plugin_slots.upsert(slot);
        }),
        update_slot: Arc::new({
            let state = state.clone();
            move |plugin_id: &PluginId, stable_id: &str, text: &str| {
                state
                    .write()
                    .plugin_slots
                    .update_slot_text(plugin_id, stable_id, text);
            }
        }),
        get_entries: Arc::new({
            let state = state.clone();
            move || -> Vec<rhai::Map> {
                let guard = state.read();
                let session = guard.active_session();
                session
                    .history()
                    .iter()
                    .map(|entry| {
                        let mut map = rhai::Map::new();
                        map.insert("kind".into(), entry.kind_str().into());
                        map.insert("text".into(), entry.text().into());
                        map
                    })
                    .collect()
            }
        }),
        send_command: Arc::new({
            let sink = sink_clone.clone();
            move |name: &str, _value: serde_json::Value| {
                if nullslop_plugin::command_allowlist::is_allowed(name) {
                    // Future: route allowlisted commands
                    let _ = sink;
                }
            }
        }),
    })
}

/// Clears all status bar slots for a plugin.
fn clear_slots_for_plugin(state: &State, plugin_id: &PluginId) {
    state.write().plugin_slots.clear_for_plugin(plugin_id);
}

/// Pushes a system chat entry to the active session.
fn push_system_entry(state: &State, message: &str) {
    let mut guard = state.write();
    guard
        .active_session_mut()
        .push_entry(ChatEntry::system(message));
}

/// Converts an `Event` directly into a rhai Map for plugin consumption.
///
/// Each match arm defines the plugin-facing schema for that event type.
/// Events without a match arm still deliver the `type` field so plugins
/// can detect unknown events.
fn event_to_rhai_map(type_name: &str, event: &Event) -> rhai::Map {
    let mut map = rhai::Map::new();
    map.insert("type".into(), type_name.into());

    match event {
        Event::ChatEntrySubmitted(payload) => {
            map.insert("session_id".into(), payload.session_id.to_string().into());
            map.insert("kind".into(), payload.entry.kind_str().into());
            map.insert("text".into(), payload.entry.text().into());
        }
        Event::StreamCompleted(payload) => {
            map.insert("session_id".into(), payload.session_id.to_string().into());
            map.insert("reason".into(), stream_reason_str(&payload.reason).into());
        }
        Event::ActiveSessionChanged(payload) => {
            map.insert("session_id".into(), payload.session_id.to_string().into());
        }
        // Events without explicit mapping still carry `type`.
        _ => {}
    }

    map
}

/// Returns a lowercase string for `StreamCompletedReason`.
fn stream_reason_str(
    reason: &crate::feat::provider::protocol::event::StreamCompletedReason,
) -> &'static str {
    match reason {
        crate::feat::provider::protocol::event::StreamCompletedReason::Finished => "finished",
        crate::feat::provider::protocol::event::StreamCompletedReason::Canceled => "canceled",
        crate::feat::provider::protocol::event::StreamCompletedReason::ToolUse => "tool_use",
    }
}
