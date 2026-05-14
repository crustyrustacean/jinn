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
    ForwardEvent(String, String),
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
            serde_json::to_string(event).unwrap_or_default(),
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
            PluginThreadMsg::ForwardEvent(type_name, event_json) => {
                let subs = event_subscriptions.lock().unwrap();
                let Some(subscribed_ids) = subs.get(&type_name) else {
                    continue;
                };
                for plugin_id in subscribed_ids {
                    if let Some(runtime) = runtimes.get_mut(plugin_id) {
                        let event_map = build_event_map(&type_name, &event_json);
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

/// Converts a serialized Event JSON into a rhai Map for plugin consumption.
///
/// The wire format is `{"VariantName": {fields...}}`. We extract the variant
/// name and flatten the fields into a single map with a `type` key.
///
/// For events containing a `ChatEntry` (inside an `entry` field), the entry's
/// `kind` is converted from the serde format `{"User": "hello"}` to a simple
/// string `"user"` and a `text` field is added.
fn build_event_map(type_name: &str, event_json: &str) -> rhai::Map {
    let mut map = rhai::Map::new();
    map.insert("type".into(), type_name.into());

    // Best-effort: parse the outer JSON and extract the inner object fields.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(event_json) {
        if let Some(obj) = value.as_object() {
            // The outer object has one key: the variant name.
            // The inner value is the payload object.
            if let Some((_, payload)) = obj.iter().next() {
                if let Some(payload_obj) = payload.as_object() {
                    for (key, val) in payload_obj {
                        if key == "entry" {
                            // Flatten ChatEntry fields for plugin convenience.
                            if let Some(entry_obj) = val.as_object() {
                                // Extract kind as a simple string.
                                let kind = entry_obj
                                    .get("kind")
                                    .and_then(|k| k.as_object())
                                    .and_then(|k| {
                                        // ChatEntryKind serializes as {"User": "hello"} etc.
                                        k.keys().next().map(|k| k.to_lowercase())
                                    })
                                    .unwrap_or_else(|| "unknown".to_owned());
                                map.insert("kind".into(), kind.into());

                                // Extract text from the kind value.
                                let text = entry_obj
                                    .get("kind")
                                    .and_then(|k| k.as_object())
                                    .and_then(|k| k.values().next())
                                    .and_then(|v| v.as_str().map(String::from))
                                    .unwrap_or_default();
                                map.insert("text".into(), text.into());
                            }
                        } else {
                            let rhai_val = json_to_rhai(val);
                            map.insert(key.clone().into(), rhai_val);
                        }
                    }
                }
            }
        }
    }

    map
}

/// Converts a serde_json Value to a rhai Dynamic value (best-effort).
fn json_to_rhai(val: &serde_json::Value) -> rhai::Dynamic {
    match val {
        serde_json::Value::Null | serde_json::Value::Object(_) => rhai::Dynamic::UNIT,
        serde_json::Value::Bool(b) => (*b).into(),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into()
            } else {
                n.as_f64().unwrap_or(0.0).into()
            }
        }
        serde_json::Value::String(s) => s.clone().into(),
        serde_json::Value::Array(arr) => {
            let mapped: Vec<rhai::Dynamic> = arr.iter().map(json_to_rhai).collect();
            rhai::Dynamic::from(mapped)
        }
    }
}
