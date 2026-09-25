//! Provider picker loader — builds provider picker entries into the
//! picker's `SelectionState`.
//!
//! Re-homed from the kernel `feat/provider/loader.rs`; its inputs are
//! explicit (cell model-cache snapshot + theme + session model snapshot), and
//! `set_endpoint_picker_items` helper moved to
//! [`crate::endpoint_loader`] as actor methods.

use jinn_core_types::ModelSelection;
use jinn_picker::PickerEntry;
use jinn_provider_config::ModelCache;
use jinn_provider_selection_msg::ProviderPickerEntry;
use jinn_selection_widget::SelectionState;

use crate::entries::load_provider_entries;
use jinn_domain::Services;
use jinn_picker::PROVIDER_ID;
use jinn_picker_specs::build_picker_registry;

/// Loads provider entries into the picker state, ready for display.
///
/// Reads from the provider registry and the model-cache snapshot, applies
/// available-first sorting and active-provider promotion, then wraps the
/// entries through the provider spec's hooks and stores them via
/// `SelectionState::set_items`.
pub(crate) fn load_provider_picker_items(
    services: &Services,
    picker: &mut SelectionState<PickerEntry<ProviderPickerEntry>>,
    model_cache: Option<&ModelCache>,
    theme: &jinn_domain::feat::theme::Theme,
    model_selection: &ModelSelection,
    alloy_mode: bool,
) {
    let registry = services.provider_registry.read();
    let api_keys = services.api_keys.read();
    let all = load_provider_entries(&registry, &api_keys, model_cache, theme);

    let active_model = model_selection.display_str().to_owned();
    let mut entries = sorted_entries(&all, "", &active_model);

    // Pre-check entries matching the current model selection, but only when
    // the picker is in alloy mode. Single mode never builds checkmarks.
    if alloy_mode {
        pre_check_active_models(&mut entries, model_selection);
        promote_selected_to_top(&mut entries);
    }

    let wrapped = build_picker_registry()
        .make_items(PROVIDER_ID, entries)
        .unwrap_or_default();
    picker.set_items(wrapped);
}

/// Sets `selected = true` on entries matching the current model selection.
///
/// For `Single`, checks the one matching entry. For `Alloy`, checks all member entries.
pub(crate) fn pre_check_active_models(
    entries: &mut [ProviderPickerEntry],
    selection: &ModelSelection,
) {
    let model_ids: Vec<&str> = match selection {
        ModelSelection::Single(s) => vec![s],
        ModelSelection::Alloy { models, .. } => models.iter().map(String::as_str).collect(),
    };
    for entry in entries.iter_mut() {
        if model_ids.iter().any(|id| *id == entry.provider_id) {
            entry.selected = true;
        }
    }
}

/// Reorders entries so that available entries appear first (sorted by model name),
/// followed by unavailable entries (sorted by model name). When `filter` is empty,
/// the entry matching `active_provider` is promoted to the very top and marked active.
///
/// `active_provider` is in `{name}/{model}` format (e.g., `"ollama/llama3"`).
fn sorted_entries(
    entries: &[ProviderPickerEntry],
    filter: &str,
    active_provider: &str,
) -> Vec<ProviderPickerEntry> {
    // Split into available and unavailable blocks.
    let mut available: Vec<ProviderPickerEntry> =
        entries.iter().filter(|e| e.is_available).cloned().collect();
    let mut unavailable: Vec<ProviderPickerEntry> = entries
        .iter()
        .filter(|e| !e.is_available)
        .cloned()
        .collect();

    // Sort each block alphabetically by model name (case-insensitive).
    available.sort_by_key(|e| e.model.to_lowercase());
    unavailable.sort_by_key(|e| e.model.to_lowercase());

    // Promote selected entries to top (for multi-select alloy building).
    promote_selected_to_top(&mut available);

    // Promote active provider to top when filter is empty.
    if filter.is_empty() && active_provider != jinn_core_types::NO_PROVIDER_ID {
        promote_active_to_top(&mut available, |e| e.provider_id == active_provider, filter);
    }

    // Mark active entries.
    for entry in &mut available {
        entry.is_active = entry.provider_id == active_provider;
    }

    // Merge: available first, then unavailable.
    available.extend(unavailable);
    available
}

/// Promotes entries with `selected == true` to the top of the list.
fn promote_selected_to_top(entries: &mut Vec<ProviderPickerEntry>) {
    // Stable partition: selected entries move to top, preserving alphabetical order within each group.
    let selected: Vec<ProviderPickerEntry> = entries.extract_if(.., |e| e.selected).collect();
    let mut result = selected;
    result.append(entries);
    *entries = result;
}

/// Promotes the active provider's entry to the top (kernel helper).
fn promote_active_to_top<T, F>(entries: &mut [T], is_active: F, filter: &str)
where
    F: Fn(&T) -> bool,
{
    jinn_picker::picker_style::promote_active_to_top(entries, is_active, filter);
}
