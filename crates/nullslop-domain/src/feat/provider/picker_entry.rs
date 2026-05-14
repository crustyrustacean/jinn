//! Provider picker entry type and rendering.

use std::ops::Range;

use crate::feat::picker::style::{active_marker, selected_style};
use nullslop_selection_widget::PickerItem;
use nullslop_selection_widget::highlight_text;
use ratatui::style::{Color, Style};
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
    let active_marker = active_marker(entry.is_active);

    let status_prefix = if !entry.is_available {
        "\u{2717} " // ✗
    } else if entry.is_alias {
        "\u{2192} " // →
    } else if entry.is_remote {
        "* "
    } else {
        "  "
    };

    let label_style = if entry.is_available {
        selected_style(is_selected)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    if entry.is_alias {
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
    if !prefix.is_empty() {
        spans.push(Span::styled(prefix.to_owned(), base_style));
    }
    spans.extend(highlight_text(model, base_style, match_indices));
    if !suffix.is_empty() {
        spans.push(Span::styled(suffix.to_owned(), base_style));
    }

    spans
}
