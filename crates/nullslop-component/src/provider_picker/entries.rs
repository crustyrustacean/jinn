//! Provider entries for the picker.
//!
//! Builds the list of providers and aliases available for selection,
//! and implements [`PickerItem`] so [`SelectionState`] can fuzzy-filter
//! and render them. Also provides footer formatting utilities for the
//! provider picker overlay.
//!
//! [`PickerItem`]: nullslop_selection_widget::PickerItem
//! [`SelectionState`]: nullslop_selection_widget::SelectionState

use std::ops::Range;

use crate::PICKER_HIGHLIGHT_STYLE;
use nullslop_selection_widget::PickerItem;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// A provider entry ready for display in the picker.
#[derive(Debug, Clone)]
pub struct PickerEntry {
    /// Full provider ID in `{name}/{model}` format (e.g., `"ollama/llama3"`).
    /// For aliases, this is the resolved target's full ID.
    /// For remote entries, this is `{provider_name}/{model}`.
    pub provider_id: String,
    /// Display name for the entry (provider block name or alias name).
    pub name: String,
    /// Provider block name (e.g., `"ollama"`). Used for display.
    pub provider_name: String,
    /// Backend type string.
    pub backend: String,
    /// Model identifier (primary display text).
    pub model: String,
    /// Whether this entry is an alias.
    pub is_alias: bool,
    /// Alias display target (e.g., `"ollama/llama3"`). Only set for aliases.
    pub alias_target: Option<String>,
    /// Whether this provider is available (API key present or keyless).
    pub is_available: bool,
    /// Whether this entry was discovered from a remote provider (not in static config).
    pub is_remote: bool,
    /// Whether this entry is the currently active provider.
    pub is_active: bool,
}

impl PickerItem for PickerEntry {
    fn display_label(&self) -> &str {
        // Use model as the primary label. Fuzzy matching via SelectionState
        // searches this plus name/backend through the matcher.
        &self.model
    }

    fn render_row(&self, is_selected: bool) -> Line<'static> {
        render_provider_row(self, is_selected, &[])
    }

    fn render_row_with_highlight(
        &self,
        is_selected: bool,
        match_indices: &[Range<usize>],
    ) -> Line<'static> {
        render_provider_row(self, is_selected, match_indices)
    }
}

/// Renders a provider picker row, optionally highlighting matched characters.
///
/// Match indices are byte offsets into `entry.model` (the `display_label`).
/// The label is built as `"{status}{model} ({provider_name})"` — we highlight
/// only the model portion.
fn render_provider_row(
    entry: &PickerEntry,
    is_selected: bool,
    match_indices: &[Range<usize>],
) -> Line<'static> {
    let active_marker = Span::styled(
        if entry.is_active { "> " } else { "  " },
        if entry.is_active {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        },
    );

    let status_prefix = if !entry.is_available {
        "\u{2717} " // ✗
    } else if entry.is_alias {
        "\u{2192} " // →
    } else if entry.is_remote {
        "* "
    } else {
        "  "
    };

    let label_style = if is_selected {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    } else if !entry.is_available {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
    };

    // Build the model + provider portions.
    // The suffix after the model differs for aliases vs non-aliases.
    if entry.is_alias {
        // Format: "{status}{name} → {model} ({provider_name})"
        // Only model bytes are matchable.
        let model_suffix = format!(" ({})", entry.provider_name);
        let model_spans = highlight_model_in_label(
            &format!("{}{} → ", status_prefix, entry.name),
            &entry.model,
            &model_suffix,
            label_style,
            match_indices,
        );
        Line::from(
            std::iter::once(active_marker)
                .chain(model_spans)
                .collect::<Vec<_>>(),
        )
    } else {
        // Format: "{status}{model} ({provider_name})"
        let model_suffix = format!(" ({})", entry.provider_name);
        let model_spans = highlight_model_in_label(
            status_prefix,
            &entry.model,
            &model_suffix,
            label_style,
            match_indices,
        );
        Line::from(
            std::iter::once(active_marker)
                .chain(model_spans)
                .collect::<Vec<_>>(),
        )
    }
}

/// Splits a label into spans, applying highlight to matched bytes within the model portion.
///
/// The rendered text is: `prefix + model + suffix`.
/// Only bytes within `model` are matchable (offsets into `model` bytes).
fn highlight_model_in_label<'a>(
    prefix: &str,
    model: &str,
    suffix: &str,
    base_style: Style,
    match_indices: &[Range<usize>],
) -> Vec<Span<'a>> {
    if match_indices.is_empty() {
        return vec![Span::styled(format!("{prefix}{model}{suffix}"), base_style)];
    }

    let mut spans = Vec::new();

    // Prefix (not searchable).
    if !prefix.is_empty() {
        spans.push(Span::styled(prefix.to_owned(), base_style));
    }

    // Model portion — split at match boundaries.
    spans.extend(highlight_text(model, base_style, match_indices));

    // Suffix (not searchable).
    if !suffix.is_empty() {
        spans.push(Span::styled(suffix.to_owned(), base_style));
    }

    spans
}

/// Splits `text` into spans, applying the highlight style to characters whose
/// byte offset falls within one of `match_indices`.
///
/// Matched characters get [`PICKER_HIGHLIGHT_STYLE`] patched onto the base style
/// (preserving the base foreground color).
fn highlight_text<'a>(
    text: &str,
    base_style: Style,
    match_indices: &[Range<usize>],
) -> Vec<Span<'a>> {
    if match_indices.is_empty() || text.is_empty() {
        return vec![Span::styled(text.to_owned(), base_style)];
    }

    let highlight_style = base_style.patch(PICKER_HIGHLIGHT_STYLE);

    let mut spans = Vec::new();
    let mut current_start = 0;
    let mut in_highlight = false;

    for (byte_off, _ch) in text.char_indices() {
        let is_matched = match_indices.iter().any(|r| r.contains(&byte_off));

        if is_matched != in_highlight {
            // Transition — emit accumulated text.
            let segment = text[current_start..byte_off].to_owned();
            if !segment.is_empty() {
                spans.push(Span::styled(
                    segment,
                    if in_highlight {
                        highlight_style
                    } else {
                        base_style
                    },
                ));
            }
            current_start = byte_off;
            in_highlight = is_matched;
        }
    }

    // Flush remaining.
    if current_start < text.len() {
        let rest = text[current_start..].to_owned();
        spans.push(Span::styled(
            rest,
            if in_highlight {
                highlight_style
            } else {
                base_style
            },
        ));
    }

    if spans.is_empty() {
        spans.push(Span::styled(text.to_owned(), base_style));
    }

    spans
}

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
        && active_provider != nullslop_providers::NO_PROVIDER_ID
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
    // Unavailable entries are never active.
    // (is_active defaults to false from load_provider_entries)

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

        // Format timestamp without fractional seconds.
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

    let total_len: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
    if total_len <= width {
        return line;
    }

    // Rebuild spans, trimming characters that overflow.
    let mut remaining = width;
    let mut spans = Vec::new();
    for span in line.spans {
        let char_count = span.content.chars().count();
        if remaining == 0 {
            break;
        }
        if char_count <= remaining {
            spans.push(span);
            remaining -= char_count;
        } else {
            let truncated: String = span.content.chars().take(remaining).collect();
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
    registry: &nullslop_providers::ProviderRegistry,
    api_keys: &nullslop_providers::ApiKeys,
    model_cache: Option<&nullslop_providers::ModelCache>,
) -> Vec<PickerEntry> {
    let mut entries = Vec::new();

    // Collect static provider IDs for collision detection.
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

    // Merge remote models from cache.
    if let Some(cache) = model_cache {
        let config = registry.config();
        for (provider_name, models) in &cache.entries {
            // Find the provider entry for backend/availability info.
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
                None => {
                    // Unknown provider in cache — still show it but mark unavailable.
                    ("unknown".to_owned(), false)
                }
            };

            for model in models {
                let provider_id = format!("{provider_name}/{model}");

                // Static wins on collision.
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

