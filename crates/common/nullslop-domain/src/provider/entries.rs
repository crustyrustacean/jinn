//! Provider entries — loading, sorting, and formatting.
//!
//! Contains loader functions, sorting, and formatting utilities for the
//! provider picker overlay. The [`PickerEntry`] struct and [`PickerItem`]
//! implementation live in `nullslop-protocol`.

use nullslop_protocol::PickerEntry;
/// Reorders entries so that available entries appear first (sorted by model name),
/// followed by unavailable entries (sorted by model name). When `filter` is empty,
/// the entry matching `active_provider` is promoted to the very top and marked active.
///
/// `active_provider` is in `{name}/{model}` format (e.g., `"ollama/llama3"`).
pub fn sorted_entries(
    entries: &[PickerEntry],
    filter: &str,
    active_provider: &str,
) -> Vec<PickerEntry> {
    // Split into available and unavailable blocks.
    let mut available: Vec<PickerEntry> =
        entries.iter().filter(|e| e.is_available).cloned().collect();
    let mut unavailable: Vec<PickerEntry> = entries
        .iter()
        .filter(|e| !e.is_available)
        .cloned()
        .collect();

    // Sort each block alphabetically by model name (case-insensitive).
    available.sort_by(|a, b| a.model.to_lowercase().cmp(&b.model.to_lowercase()));
    unavailable.sort_by(|a, b| a.model.to_lowercase().cmp(&b.model.to_lowercase()));

    // Promote active provider to top when filter is empty.
    if filter.is_empty()
        && active_provider != crate::providers::NO_PROVIDER_ID
        && let Some(pos) = available
            .iter()
            .position(|e| e.provider_id == active_provider)
        && pos > 0
    {
        #[expect(
            clippy::indexing_slicing,
            reason = "pos comes from iter().position() on the same vec"
        )]
        available[0..=pos].rotate_right(1);
    }

    // Mark active entries.
    for entry in &mut available {
        entry.is_active = entry.provider_id == active_provider;
    }

    // Merge: available first, then unavailable.
    available.extend(unavailable);
    available
}

/// Formats the footer line showing refresh keybind and last update time.
///
/// Returns a styled [`Line`] with the pipe separator in dark gray.
/// Format: `CTRL+R to refresh | Updated <timestamp> (<humantime> ago)`
pub fn format_footer(
    last_refreshed_at: Option<&jiff::Timestamp>,
    width: usize,
) -> ratatui::text::Line<'static> {
    use ratatui::style::{Color, Style};
    use ratatui::text::{Line, Span};

    let gray = Style::default().fg(Color::DarkGray);
    let orange = Style::default().fg(Color::Rgb(255, 165, 0));

    if let Some(ts) = last_refreshed_at {
        let elapsed = jiff::Timestamp::now() - *ts;
        let secs = elapsed.total(jiff::Unit::Second).unwrap_or(0.0).round() as u64;
        let duration = std::time::Duration::from_secs(secs);
        let human = humantime::format_duration(duration);
        let age_color = age_color(secs);

        let formatted_ts = format!("{ts:.0}");

        let left = "CTRL+R to refresh ";
        let pipe = "|";
        let mid = format!(" Updated {formatted_ts} (");
        let right = format!("{human} ago)");

        let line = Line::from(vec![
            Span::styled(left.to_owned(), orange),
            Span::styled(pipe.to_owned(), gray),
            Span::styled(mid, gray),
            Span::styled(right, Style::default().fg(age_color)),
        ]);
        truncate_line(line, width)
    } else {
        let left = "CTRL+R to refresh ";
        let pipe = "|";
        let right = " Updated never";

        let line = Line::from(vec![
            Span::styled(left.to_owned(), orange),
            Span::styled(pipe.to_owned(), gray),
            Span::styled(right.to_owned(), gray),
        ]);
        truncate_line(line, width)
    }
}

/// Returns the age-based color for the "time ago" text.
///
/// - `<= 2 weeks` → light green
/// - `> 2 weeks, <= 4 weeks` → yellow
/// - `> 4 weeks` → red
pub fn age_color(secs: u64) -> ratatui::style::Color {
    use ratatui::style::Color;

    const TWO_WEEKS: u64 = 14 * 24 * 60 * 60;
    const FOUR_WEEKS: u64 = 28 * 24 * 60 * 60;
    if secs <= TWO_WEEKS {
        Color::LightGreen
    } else if secs <= FOUR_WEEKS {
        Color::Yellow
    } else {
        Color::Red
    }
}

/// Truncates a styled line to fit within `width` terminal columns.
pub fn truncate_line(
    line: ratatui::text::Line<'static>,
    width: usize,
) -> ratatui::text::Line<'static> {
    use ratatui::text::{Line, Span};
    use unicode_segmentation::UnicodeSegmentation as _;

    let total_len: usize = line
        .spans
        .iter()
        .map(|s| s.content.graphemes(true).count())
        .sum();
    if total_len <= width {
        return line;
    }

    let mut remaining = width;
    let mut spans = Vec::new();
    for span in line.spans {
        let char_count = span.content.graphemes(true).count();
        if remaining == 0 {
            break;
        }
        if char_count <= remaining {
            spans.push(span);
            remaining -= char_count;
        } else {
            let truncated: String = span.content.graphemes(true).take(remaining).collect();
            spans.push(Span::styled(truncated, span.style));
            remaining = 0;
        }
    }
    Line::from(spans)
}

/// Loads all provider and alias entries from the registry, ready for `set_items()`.
///
/// Reads the provider registry, API keys, and optional model cache
/// to produce the full list of entries. No filtering is applied — that is
/// handled by [`SelectionState`] via fuzzy matching on [`PickerItem::display_label`].
///
/// Remote models from the cache are merged in after static entries. Static entries
/// win on collision (same `{provider_name}/{model}` key). Remote entries are marked
/// with `is_remote: true`.
///
/// [`SelectionState`]: nullslop_selection_widget::SelectionState
/// [`PickerItem`]: nullslop_selection_widget::PickerItem
pub fn load_provider_entries(
    registry: &crate::providers::ProviderRegistry,
    api_keys: &crate::providers::ApiKeys,
    model_cache: Option<&crate::providers::ModelCache>,
) -> Vec<PickerEntry> {
    let mut entries = Vec::new();

    let mut static_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for provider in registry.providers() {
        let entry = PickerEntry {
            provider_id: provider.id.to_string(),
            name: provider.name.clone(),
            provider_name: provider.name.clone(),
            backend: provider.backend.clone(),
            model: provider.model.clone(),
            is_alias: false,
            alias_target: None,
            is_available: registry.is_available(&provider.id.clone(), api_keys),
            is_remote: false,
            is_active: false,
        };

        static_ids.insert(entry.provider_id.clone());
        entries.push(entry);
    }

    for alias in registry.aliases() {
        let resolved = registry.resolve_alias(&alias.name);
        let is_available = resolved.is_some_and(|r| registry.is_available(&r.id.clone(), api_keys));

        let entry = PickerEntry {
            provider_id: resolved.map(|r| r.id.to_string()).unwrap_or_default(),
            name: alias.name.clone(),
            provider_name: resolved.map(|r| r.name.clone()).unwrap_or_default(),
            backend: resolved.map(|r| r.backend.clone()).unwrap_or_default(),
            model: resolved.map(|r| r.model.clone()).unwrap_or_default(),
            is_alias: true,
            alias_target: resolved.map(|r| r.id.to_string()),
            is_available,
            is_remote: false,
            is_active: false,
        };

        entries.push(entry);
    }

    if let Some(cache) = model_cache {
        let config = registry.config();
        for (provider_name, models) in &cache.entries {
            let provider_entry = config.providers.iter().find(|p| &p.name == provider_name);

            let (backend, is_available) = match provider_entry {
                Some(pe) => {
                    let avail = if pe.requires_key {
                        pe.api_key_env
                            .as_ref()
                            .is_some_and(|env| api_keys.is_set(env))
                    } else {
                        true
                    };
                    (pe.backend.clone(), avail)
                }
                None => ("unknown".to_owned(), false),
            };

            for model in models {
                let provider_id = format!("{provider_name}/{model}");

                if static_ids.contains(&provider_id) {
                    continue;
                }

                let entry = PickerEntry {
                    provider_id,
                    name: provider_name.clone(),
                    provider_name: provider_name.clone(),
                    backend: backend.clone(),
                    model: model.clone(),
                    is_alias: false,
                    alias_target: None,
                    is_available,
                    is_remote: true,
                    is_active: false,
                };

                entries.push(entry);
            }
        }
    }

    entries
}

#[cfg(test)]
#[path = "entries_tests.rs"]
mod entries_tests;
