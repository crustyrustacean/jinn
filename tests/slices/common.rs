//! Shared harness for the slice-composition integration tests (`tests/slices`).
//!
//! Tests here compose the real system: real slice activation over the
//! kernel's registries, real route rows, real cells and actors. Everything
//! the production launch path does — minus the terminal itself.
//!
//! This harness exists in the root crate's `tests/` (not inside a slice or
//! the tui crate) because that is the only place a test may depend on
//! several slice crates at once: `just check` and IDE analysis never
//! compile integration targets, so the tui crate stays slice-free.

#![allow(clippy::expect_used, clippy::panic, reason = "test harness")]

use jinn_domain::AppCore;
use jinn_sidebar::sections::register_sections;
use jinn_sidebar::sections::sidebar::Sidebar;
use jinn_tui::TuiApp;
use jinn_tui::app::WhichKeyInstance;
use jinn_tui::config::TuiConfig;
use jinn_tui::keymap;
use jinn_tui::selection::{SelectableRects, SelectionState};
use jinn_tui::suspend::Suspend;
use jinn_tui::{AppStatus, MsgHandler};

/// Builds a `TuiApp` over a **fully activated** slice system.
///
/// Drains the slices' forward-bridge routes, activates the dashboard and
/// quake-bar slices, and generates the composed keymap from every
/// attached row — the test twin of the production bootstrap
/// (`actor_wiring::build` + `launch`).
///
/// # Panics
///
/// Panics if slice activation fails — the harness cannot compose without
/// the slices it exists to test.
pub async fn launch_for_test(core: AppCore, mut services: jinn_domain::Services) -> TuiApp {
    let mut ui_registry = jinn_domain::AppUiRegistry::new();
    jinn_domain::register_all_ui_elements(&mut ui_registry);
    jinn_status_bar::register(&mut ui_registry);
    jinn_chat_input::register(&mut ui_registry);

    // Slice activation on the ambient runtime (test path is async).
    // `Services` itself is mutated: the viewport is the render-side view
    // registry and `Viewport::clone` is an empty shell by design, so
    // views must register into the instance that reaches `TuiApp`.
    let mut keymap = keymap::init();
    // The two activate calls below cannot panic directly, but the keymap
    // bootstrap after them must abort launch on a broken pairing.
    #[expect(
        clippy::panic,
        reason = "bootstrap assertion: a broken pairing must abort launch, not render blank"
    )]
    {
        let activated = jinn_dashboard::activate(&mut jinn_dashboard::SliceCtx {
            slices: &services.slices,
            key_routes: &services.key_routes,
            viewport: &mut services.viewport,
            trouper_system: &services.trouper_system,
        });
        if let Err(error) = activated {
            panic!("dashboard slice activation failed: {error}");
        }
        activate_quake_bar(&mut services);
        activate_status_bar(&mut services);
        activate_scope_focus(&mut services);
        activate_chat_input(&mut services);
        activate_cwd(&mut services);
        activate_project(&mut services);
        activate_preferences(&mut services);
        activate_sidebar(&mut services, core.state.clone()).await;
        activate_theme(&mut services);
        activate_persona(&mut services);
        activate_token_count(&mut services, core.state.clone()).await;
        activate_turn_dispatch(&mut services, core.state.clone()).await;
        activate_inference(&mut services).await;
        activate_watchdog(&mut services, &core.state).await;
        activate_citations(&mut services).await;
        // Every slice-owned picker, in the same order as `actor_wiring`.
        activate_every_picker(&mut services);
        jinn_tools::activate(&mut services, &core.state);
        core.state
            .write()
            .frontend
            .attach_slices(services.slices.clone());
        activate_session_init(&mut services, &core).await;
        // Provider-selection: mints the provider cell + spawns the
        // provider/discover actors (production wiring calls the same
        // activation before the boot trio, whose init actor writes the
        // disk-loaded cache through the returned cell).
        activate_provider_selection(&mut services, &core.state);
        // Bindings generate after all activations so every slice's rows exist.
        jinn_tui::keymap_gen::bind_route_rows(&services.key_routes, &mut keymap);
    }

    let initial_scope = jinn_tui::app::scope_for_focus(&core.state.read().frontend.scope());

    TuiApp {
        core,
        services,
        ui_registry,
        events: MsgHandler::new(),
        which_key: WhichKeyInstance::new(keymap, initial_scope),
        suspend: Suspend::new(),
        event_thread: None,
        status: AppStatus::Starting,
        selection: SelectionState::Idle,
        selectable_rects: SelectableRects::default(),
        pending_clipboard: false,
        config: TuiConfig::default(),
        sidebar: {
            let mut s = Sidebar::new();
            register_sections(&mut s);
            s
        },
    }
}

/// Activates the quake-bar slice over the kernel's registries.
///
/// The slice crate is kernel-free, so composition assembles the
/// `SliceHost` borrows and hands them over.
fn activate_quake_bar(services: &mut jinn_domain::Services) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_quake_bar::activate(&mut host);
    host.finalize(&|_scope, _hook| {});
}

fn activate_scope_focus(services: &mut jinn_domain::Services) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_scope_focus::activate(&mut host);
    host.finalize(&|_scope, _hook| {});
}

fn activate_chat_input(services: &mut jinn_domain::Services) {
    // `Services` is cheap to clone (Arc fields); the clone side-steps the
    // host's mutable viewport borrow, matching the production wiring.
    let services_snapshot = services.clone();
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    let deps = jinn_domain::common::actor_deps::ActorDeps {
        services: services_snapshot,
    };
    let state = jinn_domain::common::state::State::new(jinn_domain::AppState::default());
    jinn_chat_input::activate(&mut host, deps, &state);
    host.finalize(&|_scope, _hook| {});
}

fn activate_status_bar(services: &mut jinn_domain::Services) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_status_bar::activate(&mut host);
    host.finalize(&|_scope, _hook| {});
}

/// Activates the session-init slice over the kernel's registries and
/// drains its crossing routes.
///
/// Session-init attaches no route rows (headless discovery), so the
/// harness call is exactly the production pairing: activate, then
/// drain. The drain must complete before the first trigger publishes —
/// `launch_for_test` composes before any session exists, so ordering
/// holds by construction here.
/// Activates the provider-selection slice: mints the provider cell and
/// spawns the provider + discover actors over the same `State` and
/// trouper system the harness wires.
fn activate_provider_selection(services: &mut jinn_domain::Services, state: &jinn_domain::State) {
    let services_snapshot = services.clone();
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    let handles = jinn_provider_selection::activate(&mut host, &services_snapshot, state.clone());
    // The three pickers this slice owns, registered in the same order as
    // `actor_wiring`. The reasoning picker mints its own cell; the other two
    // reuse the cells `activate` already registered (the provider actor
    // publishes fetches into them).
    jinn_provider_selection::activate_picker(&mut host);
    jinn_provider_selection::activate_provider_picker(&mut host, &handles.provider_picker_cell);
    jinn_provider_selection::activate_endpoint_picker(&mut host, &handles.endpoint_picker_cell);
    host.finalize(&|_scope, _hook| {});
}

async fn activate_session_init(services: &mut jinn_domain::Services, core: &jinn_domain::AppCore) {
    let state = core.state.clone();
    if let Err(error) = jinn_session_init::activate(services, state) {
        panic!("session-init slice activation failed: {error}");
    }
}

/// A composed [`TuiApp`]: fake services plus every slice activated.
///
/// # Panics
///
/// Panics if slice activation fails — see [`launch_for_test`].
pub async fn test_app() -> TuiApp {
    let services = jinn_domain::Services::new_fake().await;
    let state = jinn_domain::AppState::default();
    let core = AppCore {
        state: jinn_domain::State::new(state),
        bridge: services.bridge.clone(),
    };
    launch_for_test(core, services).await
}

/// A `KeyRoutes` pre-seeded with every slice's rows, mirroring what
/// composition produces at launch (all `activate()` calls made).
///
/// # Panics
///
/// Panics if the detached quake cell cannot be minted (a fresh
/// `Slices` never has it registered, so this is unreachable).
#[must_use]
pub fn composition_routes() -> jinn_slices::route::KeyRoutes {
    let routes = jinn_slices::route::KeyRoutes::new();
    jinn_dashboard::attach_dashboard_rows(&routes);
    // The quake rows' submit/scroll actions capture a cell handle; the
    // seam mints a detached one (never registered into a live `Slices`)
    // since only row *shape* matters for keymap tests.
    let slices = jinn_slices::Slices::new();
    #[expect(
        clippy::expect_used,
        reason = "test seam: a fresh Slices never has the quake cell registered"
    )]
    let cell = slices
        .register(
            jinn_quake_bar::quake_bar_slot(),
            jinn_quake_bar::QuakeBarState::default(),
        )
        .expect("fresh Slices never has the quake cell registered");
    jinn_quake_bar::attach_quake_bar_rows(&routes, &cell);
    jinn_quake_bar::register_quake_input_hook(&routes, &cell);
    jinn_discord::attach_discord_rows(&routes);
    // The term slice's rows + capture key hook (the hook's scope shape
    // matters for hermeticity tests; the hook fn only encodes keys).
    jinn_term::route_rows::attach_rows(&routes, "<c-g>");
    jinn_term::key_hook::register(&routes);
    jinn_sidebar::key_routes::attach_sidebar_rows(&routes);
    routes
}

/// A composed keymap: built-in scope bindings + every slice's rows.
///
/// The test twin of the composed bootstrap; tests that exercise slice
/// keys query this. The per-dynamic-scope `<M-t>` toggle is spread by
/// `bind_route_rows` itself — production parity, no manual chrome here.
#[must_use]
pub fn composed_keymap() -> ratatui_which_key::Keymap<
    jinn_domain::KeyEvent,
    jinn_tui::Scope,
    jinn_domain::KernelIntent,
    jinn_tui::KeyCategory,
> {
    let routes = composition_routes();
    let mut keymap = keymap::init();
    jinn_tui::keymap_gen::bind_route_rows(&routes, &mut keymap);
    keymap
}

/// Waits (bounded) for `predicate` to hold, polling the async runtime.
///
/// Slice actors apply routed messages asynchronously; tests must wait
/// for the observable effect instead of sleeping a fixed duration.
///
/// # Panics
///
/// Panics after ~2s when the predicate never holds — the failure message
/// names what the test was waiting for.
#[expect(
    clippy::panic,
    reason = "bounded wait: a never-true predicate is a test failure"
)]
pub async fn wait_for(what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        if predicate() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for: {what}");
}

/// `wait_for` with a caller-chosen budget for tests that legitimately
/// process large message batches (the default 2s window is sized for
/// single-event propagation).
///
/// # Panics
///
/// Panics after `secs` when the predicate never holds.
#[expect(
    clippy::panic,
    reason = "bounded wait: a never-true predicate is a test failure"
)]
pub async fn wait_for_bounded(what: &str, secs: u64, mut predicate: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(secs);
    while tokio::time::Instant::now() < deadline {
        if predicate() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for: {what}");
}

/// Builds a plain (unmodified) character `KeyEvent`.
#[must_use]
pub fn plain(ch: char) -> jinn_domain::KeyEvent {
    jinn_domain::KeyEvent {
        key: jinn_domain::Key::Char(ch),
        modifiers: jinn_domain::Modifiers::none(),
    }
}

/// Activates the sidebar slice on the harness services. Async because
/// the drain awaits relay startup on the ambient tokio runtime.
pub async fn activate_sidebar(services: &mut jinn_domain::Services, state: jinn_domain::State) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_sidebar::activate(&mut host, state);
    host.finalize(&|_scope, _hook| {});
}

/// Activates the token-count slice on the harness services. Async because
/// the drain awaits relay startup on the ambient tokio runtime.
pub async fn activate_token_count(services: &mut jinn_domain::Services, state: jinn_domain::State) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    let _cache = jinn_token_count::activate(&mut host, state);
    host.finalize(&|_scope, _hook| {});
}

/// Activates the turn-dispatch slice on the harness services (the queue
/// actor + its crossing routes). Must run before anything publishes an
/// `Idle` phase event or a `DispatchTurn` — subscribe is the readiness
/// point. Async because the drain awaits relay startup on the ambient
/// tokio runtime.
pub async fn activate_turn_dispatch(
    services: &mut jinn_domain::Services,
    state: jinn_domain::State,
) {
    // `Services` is cheap to clone (Arc fields); the clone side-steps
    // the host's mutable viewport borrow for the activation call.
    let services_snapshot = services.clone();
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_turn_dispatch::activate(&mut host, state, services_snapshot);
    host.finalize(&|_scope, _hook| {});
}

/// Activates the inference slice: spawns the inference actor (trouper
/// ServiceActor) and stages its crossing routes, then drains them.
pub async fn activate_inference(services: &mut jinn_domain::Services) {
    // `Services` is cheap to clone (Arc fields); the clone side-steps
    // the host's mutable viewport borrow for the activation call.
    let services_snapshot = services.clone();
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_inference::activate(&mut host, services_snapshot);
    host.finalize(&|_scope, _hook| {});
}

/// Activates the watchdog slice: spawns the stall + tool-call watchdog
/// actors (trouper ServiceActors) on the kernel's trouper system.
///
/// Watchdog knobs are read once from the `State` snapshot (the term-slice
/// precedent); a test that wants a fast trip writes a smaller
/// `[stall_watchdog].timeout_secs` into the snapshot's preferences
/// *before* calling this helper.
pub async fn activate_watchdog(services: &mut jinn_domain::Services, state: &jinn_domain::State) {
    // `Services` is cheap to clone (Arc fields); the clone side-steps
    // the host's mutable viewport borrow for the activation call.
    let services_snapshot = services.clone();
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_watchdog::activate(&mut host, state, services_snapshot);
    host.finalize(&|_scope, _hook| {});
    eprintln!("DIAG16 watchdog activation ran");
}

/// Activates the citations slice: spawns the citations actor (trouper
/// ServiceActor) on the kernel's trouper system.
pub async fn activate_citations(services: &mut jinn_domain::Services) {
    // `Services` is cheap to clone (Arc fields); the clone side-steps
    // the host's mutable viewport borrow for the activation call.
    let services_snapshot = services.clone();
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_citations::activate(&mut host, services_snapshot);
    host.finalize(&|_scope, _hook| {});
}

pub fn activate_persona(services: &mut jinn_domain::Services) {
    // `Services::new_fake*` pre-seeds the personas cell the way production
    // wiring does; re-activating would trip the once-only slot invariant.
    if services
        .slices
        .reader::<jinn_persona_msg::Personas>(&jinn_persona_msg::personas_slot())
        .is_some()
    {
        return;
    }
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    let _scanned = jinn_persona::activate(&mut host, &services.paths.personas_dir());
    host.finalize(&|_scope, _hook| {});
}

pub fn activate_theme(services: &mut jinn_domain::Services) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_theme_slice::activate(
        &mut host,
        &services.paths.themes_dir(),
        &services.paths.system_themes_dir(),
    );
    jinn_theme_slice::activate_picker(&mut host);
    host.finalize(&|_scope, _hook| ());
}

/// Activates the cwd slice on the harness services.
pub fn activate_cwd(services: &mut jinn_domain::Services) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_cwd::activate(&mut host);
    host.finalize(&|_scope, _hook| {});
}

/// Activates the project slice on the harness services.
pub fn activate_project(services: &mut jinn_domain::Services) {
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_project::activate(&mut host);
    host.finalize(&|_scope, _hook| {});
}

/// Activates the preferences slice on the harness services.
pub fn activate_preferences(services: &mut jinn_domain::Services) {
    let system = services.trouper_system.clone();
    let services_handle = services.clone();
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_preferences::activate(
        &mut host,
        &system,
        services_handle,
        jinn_domain::common::state::State::new(
            jinn_domain::common::app_state::AppState::default_with_scope_focus(),
        ),
    );
    host.finalize(&|_scope, _hook| {});
}

/// Activates the remaining slice-owned pickers over the harness services.
///
/// The provider-selection pickers are activated by
/// [`activate_provider_selection`], which owns the cells they share.
///
/// Several pickers here register their own cell, and more than one activation
/// path calls in (persona's pre-seeded-cell branch calls this too). Registering
/// a slot twice is a wiring error, so each registration is attempted once and
/// the picker activated only when the registration succeeded.
pub fn activate_every_picker(services: &mut jinn_domain::Services) {
    use jinn_slices::cell::TypedCell;

    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );

    if slot_is_free::<jinn_persona_msg::PersonaPickerState>(
        &services.slices,
        &jinn_persona_msg::persona_picker_slot(),
    ) {
        jinn_persona::activate_picker(&mut host);
    }

    let session_cell: Option<TypedCell<jinn_session_store_msg::SessionPickerState>> = services
        .slices
        .register(
            jinn_session_store_msg::session_picker_slot(),
            jinn_session_store_msg::SessionPickerState::default(),
        )
        .ok();
    if let Some(session_cell) = session_cell {
        jinn_session_store::activate_session_picker(&mut host, &session_cell);
    }

    // These three mint their own cells inside `activate`, so they are
    // attempted only when the slot is still free.
    if slot_is_free::<jinn_tools_msg::ToolPickerState>(
        &services.slices,
        &jinn_tools_msg::tool_picker_slot(),
    ) {
        // Also activates the task-list picker this slice owns.
        jinn_tools::activate_picker(&mut host);
    }

    if slot_is_free::<jinn_skills_msg::SkillPickerState>(
        &services.slices,
        &jinn_skills_msg::skill_picker_slot(),
    ) {
        jinn_skills::activate(&mut host);
    }
    if slot_is_free::<jinn_mcp_msg::McpPickerState>(
        &services.slices,
        &jinn_mcp_msg::mcp_picker_slot(),
    ) {
        jinn_mcp_slice::activate_picker(&mut host);
    }
    if slot_is_free::<jinn_session_lifecycle_msg::SessionLifecyclePickerState>(
        &services.slices,
        &jinn_session_lifecycle_msg::session_lifecycle_picker_slot(),
    ) {
        jinn_session_lifecycle::activate_picker(&mut host);
    }

    host.finalize(&|_scope, _hook| ());
}

/// Whether `slot` holds no cell of type `T` yet.
///
/// Several pickers mint their cell inside `activate`, and more than one
/// activation path calls in. Registering a taken slot is a wiring error, so
/// each such picker is activated only when its slot is still free.
fn slot_is_free<T>(slices: &jinn_slices::Slices, slot: &jinn_slices::SlotKey) -> bool
where
    T: Send + Sync + 'static,
{
    slices.reader::<T>(slot).is_none()
}

#[cfg(test)]
mod term_keybinds_spot_check {
    use super::composed_keymap;
    use jinn_domain::{KernelIntent, Key, KeyEvent, Modifiers};
    use jinn_tui::Scope;
    use jinn_tui::app::WhichKeyInstance;

    fn wk(scope: Scope) -> WhichKeyInstance {
        WhichKeyInstance::new(composed_keymap(), scope)
    }

    fn alt_t() -> KeyEvent {
        KeyEvent {
            key: Key::Char('t'),
            modifiers: Modifiers {
                ctrl: false,
                alt: true,
                shift: false,
            },
        }
    }

    #[rstest::rstest]
    fn capture_hermeticity_and_view_binds_hold_in_composition() {
        // In capture (term:control): <M-t> stays hermetic — no overlay
        // intent fires; the key forwards to the pty like any other
        // (encoded as the ESC-prefix bytes a program expects).
        let intent = wk(Scope::Dynamic(jinn_term_msg::control_scope())).handle_key(alt_t());
        assert!(
            !matches!(&intent, Some(KernelIntent::Dynamic(d)) if d.action == "toggle-overlay"),
            "<M-t> must not toggle in capture: {intent:?}"
        );
        assert!(
            matches!(&intent, Some(KernelIntent::Dynamic(d)) if d.bytes == vec![0x1b, b't']),
            "<M-t> in capture must forward as ESC+t: {intent:?}"
        );
        // ...printable keys forward with bytes...
        let intent = wk(Scope::Dynamic(jinn_term_msg::control_scope())).handle_key(KeyEvent {
            key: Key::Char('a'),
            modifiers: Modifiers::none(),
        });
        let Some(KernelIntent::Dynamic(d)) = &intent else {
            panic!("capture must forward: {intent:?}");
        };
        assert_eq!(d.bytes, b"a".to_vec());
        // ...<c-c> forwards as ETX...
        let intent = wk(Scope::Dynamic(jinn_term_msg::control_scope())).handle_key(KeyEvent {
            key: Key::Char('c'),
            modifiers: Modifiers::ctrl(),
        });
        assert!(matches!(&intent, Some(KernelIntent::Dynamic(d)) if d.bytes == vec![0x03]));
        // ...f-keys forward...
        let intent = wk(Scope::Dynamic(jinn_term_msg::control_scope())).handle_key(KeyEvent {
            key: Key::F(4),
            modifiers: Modifiers::none(),
        });
        assert!(matches!(&intent, Some(KernelIntent::Dynamic(d)) if d.bytes == b"\x1bOS".to_vec()));
        // ...and the configured toggle beats the catch-all (handback).
        let intent = wk(Scope::Dynamic(jinn_term_msg::control_scope())).handle_key(KeyEvent {
            key: Key::Char('g'),
            modifiers: Modifiers {
                ctrl: true,
                alt: false,
                shift: false,
            },
        });
        let Some(KernelIntent::Dynamic(d)) = &intent else {
            panic!("toggle must beat catch-all: {intent:?}");
        };
        assert_eq!(d.action, "release-control");

        // In view (term:view): toggle, yank, push, chrome, T resolve.
        let view = || Scope::Dynamic(jinn_term_msg::view_scope());
        let intent = wk(view()).handle_key(alt_t());
        assert!(matches!(&intent, Some(KernelIntent::Dynamic(d)) if d.action == "toggle-overlay"));
        let intent = wk(view()).handle_key(KeyEvent {
            key: Key::Char('y'),
            modifiers: Modifiers::none(),
        });
        assert!(matches!(&intent, Some(KernelIntent::Dynamic(d)) if d.action == "yank-screen"));
        let intent = wk(view()).handle_key(KeyEvent {
            key: Key::Char('I'),
            modifiers: Modifiers::none(),
        });
        assert!(matches!(&intent, Some(KernelIntent::Dynamic(d)) if d.action == "push-screen"));
        let intent = wk(view()).handle_key(KeyEvent {
            key: Key::Char('T'),
            modifiers: Modifiers::none(),
        });
        assert!(
            matches!(&intent, Some(KernelIntent::Dynamic(d)) if d.action == "toggle-for-selected")
        );
        let intent = wk(view()).handle_key(KeyEvent {
            key: Key::Char('q'),
            modifiers: Modifiers::none(),
        });
        assert!(matches!(intent, Some(KernelIntent::Quit)));
        let intent = wk(view()).handle_key(KeyEvent {
            key: Key::Char('?'),
            modifiers: Modifiers::none(),
        });
        assert!(matches!(intent, Some(KernelIntent::ToggleWhichkey)));

        // Deliberately unbound in view: <M-`> and `i`.
        let intent = wk(view()).handle_key(KeyEvent {
            key: Key::Char('`'),
            modifiers: Modifiers {
                ctrl: false,
                alt: true,
                shift: false,
            },
        });
        assert!(
            intent.is_none(),
            "<M-`> must stay unbound in view: {intent:?}"
        );
        let intent = wk(view()).handle_key(KeyEvent {
            key: Key::Char('i'),
            modifiers: Modifiers::none(),
        });
        assert!(
            intent.is_none(),
            "`i` must stay unbound in view: {intent:?}"
        );
    }

    #[rstest::rstest]
    fn sidebar_t_resolves_to_the_session_terminal_row() {
        // Given the composed keymap queried in the sidebar sessions scope.
        let sessions = jinn_sidebar_msg::SidebarSectionId::Sessions.scope_id();
        let mut wk = wk(Scope::Dynamic(sessions));

        // When pressing 'T'.
        let intent = wk.handle_key(KeyEvent {
            key: Key::Char('T'),
            modifiers: Modifiers::none(),
        });

        // Then the session-terminal row resolves.
        assert!(
            matches!(&intent, Some(KernelIntent::Dynamic(d)) if d.action == "session-terminal"),
            "sidebar T must resolve the session-terminal row, got {intent:?}"
        );
    }

    #[rstest::rstest]
    #[test]
    fn session_terminal_row_publishes_nothing() {
        // Given the sidebar's session-terminal row dispatching in its scope.
        let routes = jinn_slices::route::KeyRoutes::new();
        jinn_sidebar::key_routes::attach_sidebar_rows(&routes);
        let sessions = jinn_sidebar_msg::SidebarSectionId::Sessions.scope_id();
        let mut state = jinn_domain::AppState::default_with_scope_focus();

        // When firing the row.
        let result = routes
            .action_for(
                &jinn_slices::DynamicIntent::new(sessions, "session-terminal", "toggle terminal"),
                jinn_slices::route::ActionCtx {
                    state: &mut state,
                    slices: &jinn_slices::Slices::new(),
                    config: jinn_slices::empty_config_layer(),
                    key_bytes: Vec::new(),
                },
            )
            .expect("session-terminal row must dispatch");

        // Then it publishes nothing. This row used to publish a
        // `KernelIntent::Dynamic` naming the term slice's action, on the
        // assumption the message would be routed back into dispatch. It is
        // not: a published message goes to the bus, and no actor subscribes
        // to `KernelIntent`, so `T` did nothing. A `RouteResult` carries no
        // local-dispatch channel, so the row now calls the term slice's
        // handler directly -- which is why the overlay opens, and why there
        // is no message here.
        assert!(
            result.messages.is_empty(),
            "session-terminal must not publish, got {:?}",
            result.message_names
        );
    }
}
