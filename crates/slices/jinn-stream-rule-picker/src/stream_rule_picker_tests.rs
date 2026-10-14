//! Observable-behavior tests for the slice-owned stream-rule picker.
//!
//! These assert what the picker *does* through its own public entry points —
//! the cell, the action functions, and the attached route rows, dispatched
//! through the real [`IntentHandler`] — never internal wiring details. A
//! test here that reached past those boundaries would break the moment the
//! implementation changed while the behavior did not.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test module, panics are acceptable"
)]

use jinn_config::{ConfigLayer, InMemoryConfigStorage};
use jinn_kernel::common::app_state::AppState;
use jinn_preferences_config::StreamRulesStorage as _;
use jinn_slices::KeyRoutes;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::EditIntent;
use jinn_stream_rule_picker_msg::PersistStreamRuleDisabled;
use jinn_stream_rule_picker_msg::{
    StreamRulePickerState, stream_rule_picker_scope, stream_rule_picker_slot,
};
use std::fmt::Write as _;
use std::sync::Arc;

use crate::stream_rule_picker_actions;
use crate::stream_rule_picker_routes::{self, STREAM_RULE_PICKER_BINDINGS};

/// A `jinn.toml` document carrying two named rules.
const TWO_RULES: &str = "[[stream_rules.entry]]\nname = \"no-todos\"\nconditions = ['TODO']\nbody = \"Finish it.\"\n\n[[stream_rules.entry]]\nname = \"no-yelling\"\nconditions = ['!!!']\nbody = \"Calm down.\"\n";

/// The sibling `stream_rules.toml` the standard harness writes: the same
/// two rules the config layer's `TWO_RULES` carries, so the assembled list
/// is deterministic — the user file replaces the embedded base set, and
/// the jinn.toml entries dedupe-replace its same-named rows in place.
const SIBLING_RULES: &str = "[[stream_rules.entry]]\nname = 'no-todos'\nconditions = ['TODO']\nscopes = ['text']\nbody = 'Finish it.'\n\n[[stream_rules.entry]]\nname = 'no-yelling'\nconditions = ['!!!']\nscopes = ['text']\nbody = 'Calm down.'\n";

/// A `jinn.toml` document carrying three named rules — the two-rule
/// standard set plus one more, so a test can watch the highlight walk past
/// the end of the standard pair.
const THREE_RULES: &str = "[[stream_rules.entry]]\nname = \"no-todos\"\nconditions = ['TODO']\nbody = \"Finish it.\"\n\n[[stream_rules.entry]]\nname = \"no-yelling\"\nconditions = ['!!!']\nbody = \"Calm down.\"\n\n[[stream_rules.entry]]\nname = \"third-rule\"\nconditions = ['FIXME']\nbody = \"Fix it.\"\n";

/// A leaked temp dir whose `jinn.toml` path has a sibling `stream_rules.toml`
/// holding [`SIBLING_RULES`] — the shape a run's preferences path takes when
/// the user keeps their rule set in the standalone file. Leaked because the
/// captured path outlives the helper and the reads never write to it.
fn temp_prefs() -> std::path::PathBuf {
    temp_prefs_with_sibling(SIBLING_RULES)
}

/// A leaked temp dir with a sibling `stream_rules.toml` written at `body`,
/// for the tests that exercise the merged-list shape.
fn temp_prefs_with_sibling(body: &str) -> std::path::PathBuf {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("jinn.toml");
    let sibling = dir
        .path()
        .join(jinn_preferences_config::STREAM_RULES_FILE_NAME);
    std::fs::write(&sibling, body).expect("write sibling rules file");
    std::mem::forget(dir);
    path
}

/// A picker cell wired to a fresh route table over a layer holding
/// `TWO_RULES`, with the run's preferences path captured like activation
/// captures it.
///
/// The cell is minted locally on its own registry — the catalog owns the
/// production registration — because these tests exercise picker *behavior*
/// directly, where the registration path is incidental.
fn wired() -> (
    TypedCell<StreamRulePickerState>,
    KeyRoutes,
    ConfigLayer,
    std::path::PathBuf,
) {
    let routes = KeyRoutes::new();
    let cell = jinn_slices::Slices::new()
        .register(stream_rule_picker_slot(), StreamRulePickerState::default())
        .expect("stream-rule picker slot is free in a fresh registry");
    let prefs = temp_prefs();
    stream_rule_picker_routes::attach_stream_rule_picker_rows(&routes, &cell, &prefs);
    stream_rule_picker_routes::register_stream_rule_picker_input_hook(&routes, &cell);
    (cell, routes, config_layer(TWO_RULES), prefs)
}

/// A slice activated through the real [`activate`] over `TWO_RULES`, with
/// its registry, route table, config layer, and preferences path returned
/// for inspection.
///
/// Tests that assert *wiring* must use this, not a hand-built stand-in: a
/// stand-in that re-implements the same calls passes even when `activate`
/// stops making them, which is exactly the regression these tests exist to
/// catch.
async fn activated() -> (
    jinn_slices::Slices,
    KeyRoutes,
    ConfigLayer,
    std::path::PathBuf,
) {
    activated_with(temp_prefs()).await
}

/// The real [`activate`] over `THREE_RULES` — the variant the toggle-list
/// tests need, where the highlight must walk three rows.
async fn activated_with_three_rules() -> (
    jinn_slices::Slices,
    KeyRoutes,
    ConfigLayer,
    std::path::PathBuf,
) {
    activated_doc(THREE_RULES, temp_prefs()).await
}

/// The real [`activate`] over a specific config document and preferences
/// path — the fully parameterized variant both helpers above delegate to.
async fn activated_doc(
    doc: &str,
    prefs: std::path::PathBuf,
) -> (
    jinn_slices::Slices,
    KeyRoutes,
    ConfigLayer,
    std::path::PathBuf,
) {
    activated_layer(config_layer(doc), prefs).await
}

/// The real [`activate`] over a caller-built config layer — the variant
/// the instrumented-backend test needs, where the dispatched layer must
/// wrap a backend the test owns.
async fn activated_layer(
    layer: ConfigLayer,
    prefs: std::path::PathBuf,
) -> (
    jinn_slices::Slices,
    KeyRoutes,
    ConfigLayer,
    std::path::PathBuf,
) {
    let slices = jinn_slices::Slices::new();
    // Production boot seeds every slice cell before any slice activates;
    // `activate` resolves the picker's cell by slot key, so the harness
    // seeds the same registry the app would.
    jinn_cell_catalog::register_all_cells(&slices);
    let key_routes = KeyRoutes::new();
    let mut viewport = jinn_slices::view::Viewport::new();
    let overlay_views = jinn_slices::OverlayViews::new();
    let services = jinn_kernel::Services::new_fake().await;
    let mut host = jinn_slices::SliceHost::new(
        &slices,
        &mut viewport,
        &overlay_views,
        &key_routes,
        &services.trouper_system,
    );

    crate::activate(&mut host, prefs.clone(), config_layer(TWO_RULES));

    (slices, key_routes, layer, prefs)
}

/// The real [`activate`] over a specific preferences path — the variant the
/// merged-list tests need, since the sibling rules file lives beside it.
async fn activated_with(
    prefs: std::path::PathBuf,
) -> (
    jinn_slices::Slices,
    KeyRoutes,
    ConfigLayer,
    std::path::PathBuf,
) {
    activated_doc(TWO_RULES, prefs).await
}

/// A `ConfigLayer` over an in-memory document — the same shape a route
/// action's `ActionCtx` lends the picker, and the same storage boundary the
/// confirm writes through.
fn config_layer(doc: &str) -> ConfigLayer {
    ConfigLayer::load(Arc::new(InMemoryConfigStorage::new(
        doc.parse().expect("test document parses"),
    )))
    .expect("layer loads")
}

/// Dispatches `action` against an activated slice through the real intent
/// handler, as a keypress would.
fn dispatch(
    action: &str,
    state: &mut AppState,
    slices: &jinn_slices::Slices,
    key_routes: &KeyRoutes,
    config: &ConfigLayer,
) -> jinn_slices::RouteResult {
    jinn_kernel::IntentHandler::handle(
        &jinn_kernel::KernelIntent::Dynamic(jinn_slices::DynamicIntent::new(
            stream_rule_picker_scope(),
            action,
            action,
        )),
        state,
        slices,
        key_routes,
        config,
    )
}

/// The names currently shown by the picker, in display order — the
/// *filtered* list, what the picker's rows actually draw.
fn visible_names(cell: &TypedCell<StreamRulePickerState>) -> Vec<String> {
    let guard = cell.read();
    (0..guard.selection.filtered_count())
        .filter_map(|idx| {
            guard
                .selection
                .filtered_item(idx)
                .map(|item| item.entry().name.clone())
        })
        .collect()
}

/// The pending disabled flag of the named row, or `None` when absent.
fn pending_of(cell: &TypedCell<StreamRulePickerState>, name: &str) -> Option<bool> {
    cell.read()
        .selection
        .items()
        .iter()
        .find(|item| item.entry().name == name)
        .map(|item| item.entry().disabled)
}

/// The persisted `disabled_rules` array of `layer`.
fn persisted_of(layer: &ConfigLayer, name: &str) -> bool {
    layer
        .get::<jinn_preferences_config::StreamRulesConfig>()
        .map(|section| section.disabled_rules.iter().any(|held| held == name))
        .unwrap_or(false)
}

/// A test-local sink recording every payload published through it, so a
/// test can assert on the command a confirm actually carried.
#[derive(Debug, Default)]
struct RecordingSink {
    payloads: std::sync::Mutex<Vec<serde_json::Value>>,
}

impl jinn_slices::PublishSink for RecordingSink {
    fn publish_schema(
        &self,
        _schema_id: trouper::schema::SchemaId,
        payload: serde_json::Value,
        _name: &'static str,
    ) {
        self.payloads.lock().expect("sink lock").push(payload);
    }
}

/// Drains `result`'s publish closures into a sink and decodes the single
/// persist command they carried, if any.
fn persist_command_of(result: jinn_slices::RouteResult) -> Option<PersistStreamRuleDisabled> {
    let sink = RecordingSink::default();
    for publish in result.messages {
        publish(&sink);
    }
    let payloads = sink.payloads.lock().expect("sink lock").clone();
    payloads
        .into_iter()
        .find_map(|payload| serde_json::from_value(payload).ok())
}

/// The rule name the picker's highlight currently sits on, if any.
fn highlighted_of(cell: &TypedCell<StreamRulePickerState>) -> Option<String> {
    stream_rule_picker_actions::highlighted_name(&cell.read())
}

// ── Open ────────────────────────────────────────────────────────────────

#[rstest::rstest]
fn opening_the_picker_lists_every_configured_rule() {
    // Given an empty picker cell.
    let (cell, _routes, config, _prefs) = wired();

    // When opening the picker over the configured rules.
    let rules = config
        .get_list::<jinn_preferences_config::schemas::StreamRuleConfig>()
        .expect("read");
    cell.update(|picker| {
        stream_rule_picker_actions::open(
            picker,
            &rules,
            &std::collections::HashSet::new(),
            &jinn_theme::default_theme(),
        );
    });

    // Then both rules are shown in configuration order.
    assert_eq!(visible_names(&cell), vec!["no-todos", "no-yelling"]);
}

#[rstest::rstest]
#[tokio::test]
async fn the_opener_dispatches_through_the_route_row_and_seeds_from_the_config() {
    // Given an activated stream-rule picker slice and default state.
    let (slices, key_routes, config, _prefs) = activated().await;
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");
    let mut state = AppState::default_with_scope_focus();

    // When handling the opener key's dynamic intent.
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then the picker's scope is on the focus stack — the handler applies
    // the row's scope signal as it dispatches.
    assert_eq!(
        state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(stream_rule_picker_scope()),
        "the opener row must push the picker's scope"
    );
    // And the picker lists the configured rules, each seeded at its
    // persisted state.
    assert_eq!(visible_names(&cell), vec!["no-todos", "no-yelling"]);
    assert_eq!(pending_of(&cell, "no-todos"), Some(false));
    assert_eq!(pending_of(&cell, "no-yelling"), Some(false));
}

#[rstest::rstest]
fn the_opener_binds_outside_the_picker_it_opens() {
    // Given the route rows the slice attaches.
    let (_cell, routes, _config, _prefs) = wired();
    let rows = routes.rows();

    // When reading the opener row.
    let opener = rows
        .iter()
        .find(|row| row.key == "<leader>sx")
        .expect("the picker has an opener key");
    let site = opener.site;

    // Then it is not an own-scope binding, which could never fire.
    assert_ne!(
        site,
        jinn_slices::route::BindSite::OwnScope,
        "a key that opens a picker cannot live inside that picker's scope"
    );
}

#[rstest::rstest]
fn every_footer_entry_is_a_key_the_picker_binds() {
    // Given the picker's declared footer binds and its bound keys.
    let bound = stream_rule_picker_routes::bound_keys();

    // When reading each footer entry.
    // Then every one is a key the picker actually attaches — a footer
    // that advertises a dead key is a bug.
    for key in STREAM_RULE_PICKER_BINDINGS.iter().map(|(key, _)| *key) {
        assert!(bound.contains(&key), "footer advertises unbound key {key}");
    }
}

#[rstest::rstest]
#[tokio::test]
async fn the_picker_registers_a_filter_input_hook() {
    // Given the wiring activation performs — the real `activate`, not a
    // hand-built stand-in, so removing the registration here is detectable.
    let (slices, key_routes, _config, _prefs) = activated().await;

    // When asking for the picker's input hook.
    let hook = key_routes.input_hook(&stream_rule_picker_scope());

    // Then one is registered, so typing reaches the filter.
    assert!(
        hook.is_some(),
        "the filter needs an input hook to receive text"
    );
    // And the cell the hook writes to is the registered one.
    assert!(
        slices
            .reader::<StreamRulePickerState>(&stream_rule_picker_slot())
            .is_some()
    );
}

#[rstest::rstest]
#[tokio::test]
async fn typing_a_character_narrows_the_visible_rules() {
    // Given an activated slice with an open picker.
    let (slices, key_routes, config, _prefs) = activated().await;
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");
    dispatch(
        "open-stream-rule-picker",
        &mut AppState::default_with_scope_focus(),
        &slices,
        &key_routes,
        &config,
    );

    // When the keymap serves an inserted character through the hook.
    let hook = key_routes
        .input_hook(&stream_rule_picker_scope())
        .expect("input hook registered at activation");
    hook(&EditIntent::InsertChar('y'));

    // Then only the rule whose search text matches remains.
    assert_eq!(visible_names(&cell), vec!["no-yelling"]);
}

// ── Merged list ─────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn opening_lists_a_rule_defined_only_in_a_sibling_rules_file() {
    // Given an activated slice whose preferences path has a sibling
    // `stream_rules.toml` the config layer knows nothing about.
    let prefs = temp_prefs_with_sibling(
        "[[stream_rules.entry]]\nname = 'from-sibling'\ndescription = 'the sibling rule'\nconditions = ['TODO']\nscopes = ['text']\nbody = 'Finish it.'\n",
    );
    let (slices, key_routes, config, _prefs) = activated_with(prefs).await;
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");

    // When the picker is opened.
    dispatch(
        "open-stream-rule-picker",
        &mut AppState::default_with_scope_focus(),
        &slices,
        &key_routes,
        &config,
    );

    // Then the sibling-file rule is listed first — the user file's set
    // replaces the embedded base and jinn.toml appends after — with the
    // jinn.toml entries following in configuration order.
    assert_eq!(
        visible_names(&cell),
        vec!["from-sibling", "no-todos", "no-yelling"]
    );
    // And it seeds at its persisted state, which is armed.
    assert_eq!(pending_of(&cell, "from-sibling"), Some(false));
}

#[rstest::rstest]
#[tokio::test]
async fn a_sibling_file_rule_toggles_and_confirms_by_name_in_the_command() {
    // Given an open picker whose first row is the sibling-file rule.
    let prefs = temp_prefs_with_sibling(
        "[[stream_rules.entry]]\nname = 'from-sibling'\nconditions = ['TODO']\nscopes = ['text']\nbody = 'Finish it.'\n",
    );
    let (slices, key_routes, config, _prefs) = activated_with(prefs).await;
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // When it is toggled and confirmed.
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    let result = dispatch(
        "confirm-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then the command names the sibling rule — the persisted form is the
    // name, whatever file defined the rule — and the picker closed without
    // an inline write.
    let command =
        persist_command_of(result).expect("a successful confirm must carry the persist command");
    assert!(
        command
            .changes
            .iter()
            .any(|change| change.name == "from-sibling" && change.disabled),
        "the sibling rule must ride the command: {:?}",
        command.changes
    );
    assert_ne!(
        state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(stream_rule_picker_scope()),
        "a successful confirm must close the picker"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn reopening_after_confirming_a_sibling_rule_shows_it_disabled() {
    // Given a preferences set where a sibling-file rule is already
    // persisted disabled — seeded directly through the layer, the way the
    // persist actor's write would leave it.
    let prefs = temp_prefs_with_sibling(
        "[[stream_rules.entry]]\nname = 'from-sibling'\nconditions = ['TODO']\nscopes = ['text']\nbody = 'Finish it.'\n",
    );
    let (slices, key_routes, config, prefs) = activated_with(prefs).await;
    config
        .set_stream_rule_disabled(&prefs, "from-sibling", true)
        .expect("seed write");
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then the sibling rule shows its persisted disabled state — the open
    // seeds from the same array the persist actor writes, whatever file
    // defined the rule.
    assert_eq!(pending_of(&cell, "from-sibling"), Some(true));
    assert_eq!(pending_of(&cell, "no-todos"), Some(false));
}

#[rstest::rstest]
#[tokio::test]
async fn a_name_in_both_a_user_file_and_jinn_toml_renders_once_as_the_enforced_definition() {
    // Given a sibling file and jinn.toml each defining `no-todos`, with
    // distinguishable descriptions.
    let prefs = temp_prefs_with_sibling(
        "[[stream_rules.entry]]\nname = 'no-todos'\ndescription = 'user file version'\nconditions = ['TODO']\nscopes = ['text']\nbody = 'Finish it.'\n",
    );
    let (slices, key_routes, config, _prefs) = activated_with(prefs).await;
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");

    // When the picker is opened.
    dispatch(
        "open-stream-rule-picker",
        &mut AppState::default_with_scope_focus(),
        &slices,
        &key_routes,
        &config,
    );

    // Then `no-todos` appears exactly once, showing the jinn.toml
    // definition — the later source in assembly order, which is the one
    // the matcher enforces (jinn.toml entries carry no description).
    let guard = cell.read();
    let no_todos: Vec<&jinn_picker::PickerEntry<jinn_stream_rule_picker_msg::StreamRuleEntry>> =
        guard
            .selection
            .items()
            .iter()
            .filter(|item| item.entry().name == "no-todos")
            .collect();
    assert_eq!(no_todos.len(), 1, "a shared name must render once");
    assert_eq!(no_todos[0].entry().description, "");
}

// ── Toggle (tab) ────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn tab_toggles_only_the_pending_state_and_touches_nothing_else() {
    // Given an open picker with the highlight on the first rule.
    let (slices, key_routes, config, _prefs) = activated().await;
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // When handling the picker's tab intent.
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then only the highlighted rule's pending state flipped, and the file
    // is untouched — the write belongs to the confirm. The highlight now
    // advances too: TAB toggles *and* walks, so it sits on the second rule.
    assert_eq!(pending_of(&cell, "no-todos"), Some(true));
    assert_eq!(pending_of(&cell, "no-yelling"), Some(false));
    assert!(!persisted_of(&config, "no-todos"));
    assert_eq!(
        highlighted_of(&cell).as_deref(),
        Some("no-yelling"),
        "a toggle must advance the highlight as well as flip the row"
    );
    // And the picker stayed open.
    assert_eq!(
        state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(stream_rule_picker_scope()),
        "a toggle must not close the picker"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn tab_on_a_moved_highlight_toggles_that_row() {
    // Given an open picker with the highlight moved to the second rule.
    let (slices, key_routes, config, _prefs) = activated().await;
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    dispatch(
        "move-stream-rule-picker-down",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // When toggling.
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then the second rule's pending state flipped and the first did not.
    assert_eq!(pending_of(&cell, "no-todos"), Some(false));
    assert_eq!(pending_of(&cell, "no-yelling"), Some(true));
}

#[rstest::rstest]
#[tokio::test]
async fn tab_twice_from_the_top_marks_the_first_two_rules_and_highlights_the_third() {
    // Given an activated slice whose config holds three rules, opened.
    let (slices, key_routes, config, _prefs) = activated_with_three_rules().await;
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // When tab is handled twice from the top of the list.
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then the first two rules are pending-disabled, the third is armed,
    // and the highlight walked onto the third row.
    assert_eq!(pending_of(&cell, "no-todos"), Some(true));
    assert_eq!(pending_of(&cell, "no-yelling"), Some(true));
    assert_eq!(pending_of(&cell, "third-rule"), Some(false));
    assert_eq!(highlighted_of(&cell).as_deref(), Some("third-rule"));
}

#[rstest::rstest]
#[tokio::test]
async fn tab_at_the_last_row_toggles_it_and_the_highlight_clamps_there() {
    // Given an open picker with the highlight moved to its last rule.
    let (slices, key_routes, config, _prefs) = activated().await;
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    dispatch(
        "move-stream-rule-picker-down",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // When tab is handled on that last row.
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then the last rule's pending state flipped and the highlight clamped
    // there — the advance is suppressed, never the toggle.
    assert_eq!(pending_of(&cell, "no-yelling"), Some(true));
    assert_eq!(highlighted_of(&cell).as_deref(), Some("no-yelling"));
}

#[rstest::rstest]
#[tokio::test]
async fn tab_toggles_but_arrow_navigation_never_does() {
    // Given an open picker over the standard two rules.
    let (slices, key_routes, config, _prefs) = activated().await;
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // When walking the highlight with the navigation keys — down, up, a
    // page down, a page up.
    for action in [
        "move-stream-rule-picker-down",
        "move-stream-rule-picker-up",
        "page-stream-rule-picker-down",
        "page-stream-rule-picker-up",
    ] {
        dispatch(action, &mut state, &slices, &key_routes, &config);
    }

    // Then every row is still armed — navigation moves, it never toggles —
    // and the highlight is back where it started.
    assert_eq!(pending_of(&cell, "no-todos"), Some(false));
    assert_eq!(pending_of(&cell, "no-yelling"), Some(false));
    assert_eq!(highlighted_of(&cell).as_deref(), Some("no-todos"));
}

// ── Cancel (esc) ────────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn cancel_closes_the_picker_and_writes_nothing() {
    // Given an open picker with one rule toggled pending-off.
    let (slices, key_routes, config, _prefs) = activated().await;
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    assert_eq!(
        pending_of(&cell, "no-todos"),
        Some(true),
        "setup: the toggle ran"
    );

    // When handling the picker's esc intent.
    dispatch(
        "cancel-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then the picker closed.
    assert_ne!(
        state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(stream_rule_picker_scope()),
        "cancel must close the picker"
    );
    // And nothing reached the file — the pending toggles died with the
    // picker.
    assert!(!persisted_of(&config, "no-todos"));
    assert!(!persisted_of(&config, "no-yelling"));
}

#[rstest::rstest]
#[tokio::test]
async fn esc_after_two_tab_toggles_persists_nothing() {
    // Given an activated slice whose config holds three rules, opened, and
    // walked by two tabs.
    let (slices, key_routes, config, _prefs) = activated_with_three_rules().await;
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    assert_eq!(
        pending_of(&cell, "no-todos"),
        Some(true),
        "setup: the first tab toggled"
    );
    assert_eq!(
        pending_of(&cell, "no-yelling"),
        Some(true),
        "setup: the second tab toggled"
    );

    // When handling the picker's esc intent.
    dispatch(
        "cancel-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then the picker closed and nothing reached the file — both pending
    // toggles died with the picker.
    assert_ne!(
        state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(stream_rule_picker_scope()),
        "cancel must close the picker"
    );
    assert!(!persisted_of(&config, "no-todos"));
    assert!(!persisted_of(&config, "no-yelling"));
    assert!(!persisted_of(&config, "third-rule"));
}

#[rstest::rstest]
#[tokio::test]
async fn reopening_after_a_cancel_shows_the_persisted_state() {
    // Given a picker whose toggle was cancelled.
    let (slices, key_routes, config, _prefs) = activated().await;
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    dispatch(
        "cancel-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // When the picker is reopened.
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then every row is seeded back at its persisted (armed) state — a
    // fresh open never inherits the cancelled edit.
    assert_eq!(pending_of(&cell, "no-todos"), Some(false));
    assert_eq!(pending_of(&cell, "no-yelling"), Some(false));
}

// ── Confirm (enter) ─────────────────────────────────────────────────────

#[rstest::rstest]
#[tokio::test]
async fn confirm_performs_no_inline_write_and_carries_the_persist_command() {
    // Given an open picker with the first rule toggled pending-off.
    let (slices, key_routes, config, prefs) = activated().await;
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // When handling the picker's enter intent.
    let result = dispatch(
        "confirm-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then the file is byte-unchanged at that instant — the write is the
    // persist actor's job, not the key handler's.
    assert!(
        !persisted_of(&config, "no-todos"),
        "the confirm itself must not write the disabled set"
    );
    // And the result carries the persist command naming the toggled rule,
    // the preferences path, and the session active at confirm time.
    let command =
        persist_command_of(result).expect("a successful confirm must carry the persist command");
    assert_eq!(command.changes.len(), 2, "every row rides the command");
    assert_eq!(command.changes[0].name, "no-todos");
    assert!(command.changes[0].disabled);
    assert_eq!(command.preferences_path, prefs);
    assert_eq!(
        command.session_id,
        state.session.active_session_id().clone()
    );
}

#[rstest::rstest]
#[tokio::test]
async fn confirm_closes_the_picker_over_the_pending_set() {
    // Given an open picker with the first rule toggled pending-off.
    let (slices, key_routes, config, _prefs) = activated().await;
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // When confirming.
    dispatch(
        "confirm-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then the picker closed: the confirm is finished even though the
    // writes are still in flight in the persist actor.
    assert_ne!(
        state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(stream_rule_picker_scope()),
        "a successful confirm must close the picker"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn enter_after_two_tab_toggles_persists_exactly_the_two_marked_rules() {
    // Given an activated slice whose config holds three rules, opened, and
    // walked by two tabs from the top.
    let (slices, key_routes, config, _prefs) = activated_with_three_rules().await;
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // When handling the picker's enter intent.
    let result = dispatch(
        "confirm-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then exactly the two marked rules ride the persist command disabled
    // and the third rides it armed — the command is what persists now, the
    // persist actor its writer, so the marked set must be complete in the
    // command the confirm carries.
    let command =
        persist_command_of(result).expect("a successful confirm must carry the persist command");
    let disabled: Vec<&str> = command
        .changes
        .iter()
        .filter(|change| change.disabled)
        .map(|change| change.name.as_str())
        .collect();
    assert_eq!(disabled, vec!["no-todos", "no-yelling"]);
    assert!(
        command
            .changes
            .iter()
            .any(|change| change.name == "third-rule" && !change.disabled),
        "the unmarked third rule rides the command armed: {:?}",
        command.changes
    );
    // And the picker closed.
    assert_ne!(
        state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(stream_rule_picker_scope()),
        "a successful confirm must close the picker"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn confirm_leaves_a_status_line_in_the_cell() {
    // Given an open picker with one rule toggled pending-off.
    let (slices, key_routes, config, _prefs) = activated().await;
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // When confirming.
    dispatch(
        "confirm-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then the cell's status line names what the set now is — the visible
    // feedback a confirm leaves behind.
    let status = cell.read().status.clone();
    assert_eq!(
        status.as_deref(),
        Some("stream rules: disabled no-todos (takes effect shortly)"),
        "confirm must report the outcome in the picker's status line"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn confirm_publishes_exactly_the_persist_command() {
    // Given an open picker with one rule toggled pending-off.
    let (slices, key_routes, config, _prefs) = activated().await;
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // When confirming.
    let result = dispatch(
        "confirm-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then the result carries exactly the persist command and nothing
    // else — in particular no matcher refresh: the refresh belongs to
    // the persist actor, published only after every write landed, and a
    // refresh from here would recompile the matcher before the write.
    assert_eq!(
        result.message_names,
        vec!["PersistStreamRuleDisabled"],
        "the persist command is the only thing a confirm publishes"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn reopening_after_a_confirm_shows_the_persisted_state() {
    // Given a preferences set with `no-todos` already persisted disabled —
    // seeded directly through the layer, the way the persist actor's write
    // would leave it.
    let (slices, key_routes, config, prefs) = activated().await;
    config
        .set_stream_rule_disabled(&prefs, "no-todos", true)
        .expect("seed write");
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then the persisted rule shows disabled on the fresh open — the open
    // seeds from the same layer the persist actor writes, so what the file
    // holds is what the list shows.
    assert_eq!(pending_of(&cell, "no-todos"), Some(true));
    assert_eq!(pending_of(&cell, "no-yelling"), Some(false));
}

#[rstest::rstest]
#[tokio::test]
async fn a_stale_name_confirm_publishes_the_command_and_closes_the_picker() {
    // Given a picker whose rows were reseeded over a config that no longer
    // holds them — the file changed underneath the open picker. This test
    // pins where the old pre-check's guarantees now live: a stale-name
    // confirm publishes the command and closes like any other confirm; the
    // unknown-name refusal and its chat-error report are the persist
    // actor's (covered by the storage and actor suites), never the
    // picker's. Do not reintroduce a picker-side check "for UX" — it was
    // the N-assemblies-per-confirm hitch this slice existed to remove.
    let (slices, key_routes, config, _prefs) = activated().await;
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    cell.update(|picker| {
        let mut rules = config
            .get_list::<jinn_preferences_config::schemas::StreamRuleConfig>()
            .expect("read");
        rules[0].name = "renamed-under-us".to_owned();
        picker
            .selection
            .set_items(jinn_stream_rule_picker_msg::build_stream_rule_entries(
                &rules,
                &std::collections::HashSet::new(),
                &picker.theme,
            ));
    });
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // When confirming — the write targets an entry that is not there, and
    // the failure is the actor's to report.
    let result = dispatch(
        "confirm-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then the command rode out — the actor will refuse the unknown name
    // off-thread and report it in chat.
    let command =
        persist_command_of(result).expect("a stale-name confirm must still carry the command");
    assert!(
        command
            .changes
            .iter()
            .any(|change| change.name == "renamed-under-us" && change.disabled),
        "the stale name must ride the command: {:?}",
        command.changes
    );
    // And the picker closed like any other confirm.
    assert_ne!(
        state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(stream_rule_picker_scope()),
        "a stale-name confirm must close the picker; its failure reports in chat"
    );
    // And nothing was written — the persist actor owns the write, and it
    // will refuse this one.
    assert!(!persisted_of(&config, "no-todos"));
    assert!(!persisted_of(&config, "no-yelling"));
}

// ── Confirm cost ────────────────────────────────────────────────────────

/// A backend that counts every `read` — the instrumented seam proving the
/// confirm's zero-I/O property.
///
/// It delegates to an [`InMemoryConfigStorage`]: the counter observes the
/// same call the real filesystem backend would receive, and the document
/// behaves normally underneath.
#[derive(Debug)]
struct CountingStorage {
    inner: InMemoryConfigStorage,
    reads: std::sync::atomic::AtomicUsize,
}

impl CountingStorage {
    fn new(doc: toml_edit::DocumentMut) -> Self {
        Self {
            inner: InMemoryConfigStorage::new(doc),
            reads: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Reads counted so far, and the mutable view a test resets through.
    fn reads(&self) -> usize {
        self.reads.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn reset(&self) {
        self.reads.store(0, std::sync::atomic::Ordering::SeqCst);
    }
}

impl jinn_config::ConfigDocumentStorage for CountingStorage {
    fn name(&self) -> &'static str {
        "CountingStorage"
    }

    fn read(
        &self,
    ) -> Result<toml_edit::DocumentMut, error_stack::Report<jinn_config::ConfigError>> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.read()
    }

    fn write(
        &self,
        doc: &toml_edit::DocumentMut,
    ) -> Result<(), error_stack::Report<jinn_config::ConfigError>> {
        self.inner.write(doc)
    }
}

/// A counting layer seeded over [`TWO_RULES`], with its backend returned.
fn counting_layer() -> (ConfigLayer, Arc<CountingStorage>) {
    let backend = Arc::new(CountingStorage::new(
        TWO_RULES.parse().expect("test document parses"),
    ));
    let layer = ConfigLayer::load(backend.clone()).expect("layer loads");
    (layer, backend)
}

#[rstest::rstest]
#[tokio::test]
async fn confirm_never_reads_the_config_layer() {
    // Given an activated slice whose dispatched layer wraps a counting
    // backend, with a picker open and one rule toggled pending-off.
    let (layer, backend) = counting_layer();
    let (slices, key_routes, config, _prefs) = activated_layer(layer, temp_prefs()).await;
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    // The open legitimately reads through the layer; the assertion window
    // starts after it, so the count it accrued is discarded here.
    backend.reset();

    // When confirming.
    let result = dispatch(
        "confirm-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then the backend's read was never consulted — the confirm reads the
    // picker's own cell, builds the command, and hands off. An earlier
    // revision assembled the whole rule set here once per pending row
    // (a full template parse per row), which this counter would have
    // caught as N reads.
    assert_eq!(
        backend.reads(),
        0,
        "the confirm must not read the config layer at all"
    );
    // And the command carried anyway — the proof is of a cheap confirm,
    // not a broken one.
    assert!(
        persist_command_of(result).is_some(),
        "a successful confirm must still carry the persist command"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn confirming_a_large_rule_set_still_closes_with_the_command() {
    // Given an activated slice whose jinn.toml carries 120 rules — larger
    // than the shipped set — opened over an empty sibling file so the
    // assembled list is exactly those 120. The assertion is functional,
    // never wall-clock: timing tests are flaky under the per-test timeout,
    // and the zero-read proof above is the real cost guarantee.
    let mut doc = String::new();
    for i in 0..120 {
        let _ = write!(
            doc,
            "[[stream_rules.entry]]\nname = \"bulk-rule-{i}\"\nconditions = ['FIXME']\nbody = \"Fix it.\"\n\n"
        );
    }
    let (slices, key_routes, config, _prefs) =
        activated_doc(&doc, temp_prefs_with_sibling("")).await;
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");
    let mut state = AppState::default_with_scope_focus();
    dispatch(
        "open-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    assert_eq!(
        visible_names(&cell).len(),
        120,
        "setup: the large set is listed"
    );

    // When two rules are toggled and the picker confirmed.
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    dispatch(
        "toggle-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );
    let result = dispatch(
        "confirm-stream-rule-picker",
        &mut state,
        &slices,
        &key_routes,
        &config,
    );

    // Then the picker closed and the command carried the whole set with
    // exactly the two marked rules disabled — a confirm over a large set
    // completes in one handoff, whatever the list size.
    let command =
        persist_command_of(result).expect("a successful confirm must carry the persist command");
    assert_eq!(command.changes.len(), 120, "every row rides the command");
    assert_eq!(
        command
            .changes
            .iter()
            .filter(|change| change.disabled)
            .map(|change| change.name.as_str())
            .collect::<Vec<_>>(),
        vec!["bulk-rule-0", "bulk-rule-1"],
        "exactly the two marked rules ride the command disabled"
    );
    assert_ne!(
        state.frontend.scope(),
        jinn_slices::FocusScope::Dynamic(stream_rule_picker_scope()),
        "a large-set confirm must close the picker"
    );
}

// ── Wiring ──────────────────────────────────────────────────────────────

#[rstest::rstest]
fn the_picker_binds_navigation_toggle_confirm_and_cancel() {
    // Given the route rows the slice attaches.
    let (_cell, routes, _config, _prefs) = wired();
    let keys: Vec<&str> = routes.rows().iter().map(|row| row.key).collect();

    // Then the picker walks, pages, toggles, confirms, and cancels.
    for key in [
        "<up>", "<down>", "<pgup>", "<pgdn>", "<tab>", "<enter>", "<esc>",
    ] {
        assert!(
            keys.contains(&key),
            "no binding for {key} (binds: {keys:?})"
        );
    }
}

#[rstest::rstest]
fn the_picker_state_lives_under_one_slot() {
    // Given a picker registered under its slot.
    let (cell, _routes, _config, _prefs) = wired();

    // When a value is written to it.
    cell.update(|picker| {
        picker.results_viewport = 7;
    });

    // Then it reads back through the same cell.
    assert_eq!(cell.read().results_viewport, 7);
    // And the slot is stable, so there is exactly one home for the
    // picker's state.
    assert_eq!(stream_rule_picker_slot(), stream_rule_picker_slot());
}

// ── Render ──────────────────────────────────────────────────────────────

/// Renders one frame of the picker from `slices`' registered overlay path,
/// returning the screen text.
fn rendered_screen(slices: &jinn_slices::Slices) -> String {
    let facts = jinn_slices::RenderFacts::new(jinn_theme::default_theme(), slices);
    let area = ratatui::layout::Rect::new(0, 0, 100, 30);
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(area.width, area.height))
            .expect("terminal");
    terminal
        .draw(|frame| {
            let popup =
                crate::stream_rule_picker_overlay_rect(&area).expect("geometry yields a rect");
            crate::render_stream_rule_picker(frame, popup, &facts);
        })
        .expect("draw");
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect()
}

#[rstest::rstest]
#[tokio::test]
async fn render_draws_an_armed_rule_with_its_enabled_mark() {
    // Given an activated slice whose picker holds one armed rule.
    let (slices, _key_routes, _config, _prefs) = activated().await;
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");
    cell.update(|picker| {
        stream_rule_picker_actions::open(
            picker,
            &[jinn_preferences_config::schemas::StreamRuleConfig {
                name: "no-todos".to_owned(),
                description: "no TODOs".to_owned(),
                conditions: vec!["TODO".to_owned()],
                body: "Finish it.".to_owned(),
                ..Default::default()
            }],
            &std::collections::HashSet::new(),
            &jinn_theme::default_theme(),
        );
    });

    // When the overlay view renders the picker.
    let screen = rendered_screen(&slices);

    // Then the armed rule's row shows the enabled mark and the name.
    assert!(
        screen.contains('\u{2713}') && screen.contains("no-todos"),
        "an armed rule must render with the enabled mark; got {screen:?}"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn render_draws_a_disabled_rule_with_its_disabled_mark() {
    // Given an activated slice whose picker holds one switched-off rule.
    let (slices, _key_routes, _config, _prefs) = activated().await;
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");
    cell.update(|picker| {
        stream_rule_picker_actions::open(
            picker,
            &[jinn_preferences_config::schemas::StreamRuleConfig {
                name: "no-yelling".to_owned(),
                description: "no !!!".to_owned(),
                conditions: vec!["!!!".to_owned()],
                body: "Calm down.".to_owned(),
                ..Default::default()
            }],
            // The persisted array names the rule, so the row renders as
            // switched off.
            &std::collections::HashSet::from(["no-yelling".to_owned()]),
            &jinn_theme::default_theme(),
        );
    });

    // When the overlay view renders the picker.
    let screen = rendered_screen(&slices);

    // Then the disabled rule's row shows the disabled mark, not the
    // enabled one.
    assert!(
        screen.contains('\u{2717}') && screen.contains("no-yelling"),
        "a disabled rule must render with the disabled mark; got {screen:?}"
    );
    assert!(!screen.contains('\u{2713}'), "got {screen:?}");
}

#[rstest::rstest]
#[tokio::test]
async fn render_draws_the_confirm_status_line() {
    // Given an activated slice whose picker carries a confirm's status line.
    let (slices, _key_routes, _config, _prefs) = activated().await;
    let cell: TypedCell<StreamRulePickerState> = slices
        .reader(&stream_rule_picker_slot())
        .expect("stream-rule picker cell registered at activation");
    cell.update(|picker| {
        stream_rule_picker_actions::open(
            picker,
            &[],
            &std::collections::HashSet::new(),
            &jinn_theme::default_theme(),
        );
        picker.status = Some("stream rules: disabled no-todos (takes effect shortly)".to_owned());
    });

    // When the overlay view renders the picker.
    let screen = rendered_screen(&slices);

    // Then the status line is on screen — the confirm's feedback reaches
    // the user.
    assert!(
        screen.contains("takes effect shortly"),
        "the confirm's status must render; got {screen:?}"
    );
}
