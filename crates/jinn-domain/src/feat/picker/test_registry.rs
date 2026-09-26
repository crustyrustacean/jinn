//! Test-only picker registry for the kernel's dispatch tests.
//!
//! The real specs live in `jinn-picker-specs`, which depends on this crate —
//! so the kernel cannot import it, not even as a dev-dependency. A
//! dev-dependency cycle is unsound here rather than merely undesirable: Cargo
//! builds two copies of `jinn-domain` in one test binary, and the specs'
//! `state_any` downcasts then target a *different* `AppState` than the host
//! lends, failing at runtime.
//!
//! These tests exercise the generic dispatch layer — scope transitions,
//! geometry, filtering, and selection lending — not any spec's
//! confirm/preview behavior. Registering specs that mirror each real spec's
//! *metadata* (widget flavor and footer-row count) keeps them testing the
//! dispatch they were written for.
//!
//! Spec *behavior* (confirm writes, open hooks, previews) is tested in
//! `jinn-picker-specs`, where the specs and this crate's public API are both
//! available.

use jinn_picker::{PickerId, PickerRegistry, PickerSpec, PickerWidget};

/// A registry carrying one spec per picker id, mirroring each real spec's
/// widget flavor and status-row presence — the two properties geometry and
/// dispatch read off the registry.
#[must_use]
pub(crate) fn test_registry() -> PickerRegistry {
    let mut registry = PickerRegistry::new();
    for (id, widget, has_status) in specs() {
        let mut spec = PickerSpec::<Entry>::new(PickerId::new(id)).widget(widget);
        if has_status {
            spec = spec.status(|_| None);
        }
        registry.register(spec);
    }
    registry
}

/// The `(id, widget, has status row)` table, mirroring the real specs.
/// The project spec — the last one the central registry still holds — declares
/// no status row.
fn specs() -> [(&'static str, PickerWidget, bool); 1] {
    use PickerWidget::List;
    [(jinn_picker::PROJECT_ID, List, false)]
}

/// A stand-in entry type: the dispatch tests never render or wrap these, they
/// only need a concrete `TreeItem` to register a spec over.
#[derive(Debug, Clone)]
pub(crate) struct Entry;

impl jinn_selection_widget::TreeItem for Entry {
    fn id(&self) -> &'static str {
        "entry"
    }

    fn parent_id(&self) -> Option<&'static str> {
        None
    }

    fn display_label(&self) -> &'static str {
        "entry"
    }

    fn render_row(&self, _is_selected: bool) -> ratatui::text::Line<'static> {
        ratatui::text::Line::raw("entry")
    }

    fn render_row_with_highlight(
        &self,
        _is_selected: bool,
        _match_indices: &[std::ops::Range<usize>],
    ) -> ratatui::text::Line<'static> {
        ratatui::text::Line::raw("entry")
    }
}
