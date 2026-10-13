//! The stream-rule picker — the `<leader>sx` picker over `[[stream_rules.entry]]`.
//!
//! Lists every configured stream rule with its armed/disabled mark, toggles
//! the highlighted rule's pending state with `<tab>`, persists the pending
//! set through the preferences-config storage boundary with `<enter>`, and
//! closes with `<esc>` writing nothing. Applies globally: one keybind, no
//! per-session state.
//!
//! The picker is fully slice-owned: its state is a cell, its keys are route
//! rows attached at activation, and its renderer reads the cell. Portable
//! values and the crossing contracts live in `jinn-stream-rule-picker-msg`.
//!
//! # What the confirm reaches
//!
//! `ActionCtx::config` — the same `jinn.toml` layer every route action
//! carries — through
//! [`jinn_preferences_config::StreamRulesStorage::set_stream_rule_disabled`],
//! which patches the on-disk document entry-by-entry. A confirmed toggle is
//! therefore on disk before the picker closes, but the *live* matcher cell
//! is not refreshed: the stream-rules slice reads its rules once at
//! activation and nothing publishes a matcher re-install. A confirmed
//! disable takes effect at the next launch; nothing else observes the write
//! mid-session.

pub mod stream_rule_picker_actions;
pub mod stream_rule_picker_render;
pub mod stream_rule_picker_routes;
pub mod stream_rule_picker_viewport;

pub use stream_rule_picker_actions::{highlighted_name, open};
pub use stream_rule_picker_render::{render_stream_rule_picker, stream_rule_picker_overlay_rect};
pub use stream_rule_picker_routes::{
    STREAM_RULE_PICKER_BINDINGS, attach_stream_rule_picker_rows,
    register_stream_rule_picker_input_hook,
};
// The picker's identity lives in the `-msg` crate beside the state it
// names; re-exported so slice consumers import it from here like the
// workflows slice's `workflow_picker_scope`.
pub use jinn_stream_rule_picker_msg::stream_rule_picker_scope;

/// Activates the slice: registers the picker's overlay and attaches its
/// route rows.
///
/// The picker's cell is not minted here: the shared cell catalog
/// (`jinn_cell_catalog::register_all_cells`) registers every slice cell in
/// one place, before any slice activates, so this resolves the handle the
/// same way every other consumer does.
///
/// # Panics
///
/// Panics if the catalog has not run — the overlay would resolve no state
/// and render nothing, and the input hook would swallow keystrokes.
#[expect(
    clippy::expect_used,
    reason = "bootstrap assertion: broken slice wiring must abort launch, not continue degraded"
)]
pub fn activate(host: &mut jinn_slices::SliceHost<'_, jinn_slices::RenderFacts>) {
    let cell = host
        .slices()
        .reader::<jinn_stream_rule_picker_msg::StreamRulePickerState>(
            &jinn_stream_rule_picker_msg::stream_rule_picker_slot(),
        )
        .expect(
            "the cell catalog registers the stream-rule picker slot before any slice activates",
        );
    let scope = stream_rule_picker_scope();
    host.register_overlay(
        scope.clone(),
        std::sync::Arc::new(stream_rule_picker_overlay_rect),
    );
    host.register_overlay_selectable(&scope);
    host.register_overlay_slot(
        scope.clone(),
        jinn_stream_rule_picker_msg::stream_rule_picker_slot(),
    );
    host.register_overlay_view(scope, std::sync::Arc::new(render_stream_rule_picker));

    // The picker's keys, and the filter's input hook, are the slice's own.
    attach_stream_rule_picker_rows(host.key_routes(), &cell);
    register_stream_rule_picker_input_hook(host.key_routes(), &cell);
}

#[cfg(test)]
mod stream_rule_picker_tests;
