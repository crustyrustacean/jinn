//! The stream-rule picker's state cell payload and its entry type.
//!
//! The picker is owned entirely by the stream-rule-picker slice: this type
//! is what its cell holds, so no other crate names a field for it.

use jinn_picker::RowCtx;
use jinn_picker::picker_style::{active_marker, dim_style, selected_style};
use jinn_preferences_config::schemas::StreamRuleConfig;
use jinn_selection_widget::highlight_text_with_bg;
use jinn_theme::Theme;
use ratatui::text::{Line, Span};

/// A stream-rule entry ready for display in the stream-rule picker.
///
/// `disabled` is the *pending* state the toggle edits: it starts at the
/// persisted value when the picker opens, and reaches the file only when
/// the picker is confirmed.
#[derive(Debug, Clone)]
pub struct StreamRuleEntry {
    /// The rule's name — its identity in `[[stream_rules.entry]]`, and what
    /// the confirm writes through the storage boundary.
    pub name: String,
    /// The rule's prose description, shown dimmed after the name.
    pub description: String,
    /// Whether the rule is switched off, as the picker's pending edit holds it.
    pub disabled: bool,
    /// Theme for styling.
    pub theme: Theme,
}

impl StreamRuleEntry {
    /// Builds one entry from a configured rule.
    #[must_use]
    pub fn from_config(rule: &StreamRuleConfig, theme: &Theme) -> Self {
        Self {
            name: rule.name.clone(),
            description: rule.description.clone(),
            disabled: rule.disabled,
            theme: theme.clone(),
        }
    }
}

/// Renders one stream-rule picker row: the cursor marker, the
/// enabled/disabled mark, the rule name, and the description after an
/// em-dash separator.
///
/// The mark is the whole point of the picker — `✓` for an armed rule,
/// `✗` for one the pending set has switched off — so it leads the row,
/// the same place the skill picker puts its enabled marker.
#[must_use]
pub fn stream_rule_row(entry: &StreamRuleEntry, ctx: &RowCtx<'_>) -> Line<'static> {
    let base_style = selected_style(ctx.is_selected, &entry.theme);
    let desc_style = dim_style(ctx.is_selected, &entry.theme);

    let mut spans = vec![active_marker(ctx.is_selected, &entry.theme)];

    let (marker, marker_color) = if entry.disabled {
        ("\u{2717} ", entry.theme.error_text) // ✗
    } else {
        ("\u{2713} ", entry.theme.focus_accent) // ✓
    };
    spans.push(Span::styled(
        marker.to_owned(),
        ratatui::style::Style::default().fg(marker_color),
    ));

    if ctx.match_ranges.is_empty() {
        spans.push(Span::styled(entry.name.clone(), base_style));
    } else {
        spans.extend(highlight_text_with_bg(
            &entry.name,
            base_style,
            ctx.match_ranges,
            entry.theme.picker_highlight_bg,
        ));
    }

    spans.push(Span::styled(
        format!(" \u{2014} {}", entry.description),
        desc_style,
    ));

    Line::from(spans)
}

impl jinn_selection_widget::TreeItem for StreamRuleEntry {
    fn id(&self) -> &str {
        &self.name
    }

    fn parent_id(&self) -> Option<&str> {
        None
    }

    fn display_label(&self) -> &str {
        &self.name
    }

    fn render_row(&self, _is_selected: bool) -> Line<'static> {
        // Rows render through the spec's row hook via PickerEntry; this
        // impl only supplies tree structure (id/parent_id) and filter text.
        Line::raw(self.display_label().to_owned())
    }

    fn render_row_with_highlight(
        &self,
        _is_selected: bool,
        _match_indices: &[std::ops::Range<usize>],
    ) -> Line<'static> {
        Line::raw(self.display_label().to_owned())
    }
}

/// The stream-rule picker's complete state: the rows, the filter, the
/// highlight, and the status line the confirm and cancel write to.
#[derive(Debug)]
pub struct StreamRulePickerState {
    /// The selection/filter state backing the picker's rows.
    pub selection: jinn_selection_widget::SelectionState<jinn_picker::PickerEntry<StreamRuleEntry>>,
    /// How many result rows fit on screen, measured by the render pass.
    pub results_viewport: usize,
    /// The theme the rows were built with, so a repaint recolors them
    /// consistently instead of stranding them on a stale palette.
    pub theme: Theme,
    /// One line of feedback (e.g. `disabled 'no-todos'`), or `None`.
    ///
    /// Written by the confirm when it persists — including when the write
    /// fails, where it carries the reason rather than letting a failed
    /// toggle look like a successful one. Cleared on the next open.
    pub status: Option<String>,
}

impl Default for StreamRulePickerState {
    fn default() -> Self {
        Self {
            selection: jinn_selection_widget::SelectionState::new(),
            // Matches the other pickers' fallback, so the very first
            // keypress before any render behaves as it does everywhere.
            results_viewport: crate::picker_scope::RESULTS_VIEWPORT_FALLBACK,
            theme: jinn_theme::default_theme(),
            status: None,
        }
    }
}

impl StreamRulePickerState {
    /// Clears the filter, moves the highlight back to the top, and drops
    /// the last status line, ready for a fresh open.
    pub fn reset(&mut self) {
        self.selection.clear_filter();
        self.selection.move_up(usize::MAX);
        self.status = None;
    }
}

/// The row's search text: name plus description, so filtering on either
/// finds the rule.
#[must_use]
pub fn stream_rule_search_text(entry: &StreamRuleEntry) -> String {
    format!("{} {}", entry.name, entry.description)
}

/// Builds picker entries from the configured rules, in the order the user
/// wrote them in `jinn.toml` — the same order the matcher fires in, so the
/// list reads like the file it edits.
#[must_use]
pub fn build_stream_rule_entries(
    rules: &[StreamRuleConfig],
    theme: &Theme,
) -> Vec<jinn_picker::PickerEntry<StreamRuleEntry>> {
    let entries: Vec<StreamRuleEntry> = rules
        .iter()
        .map(|rule| StreamRuleEntry::from_config(rule, theme))
        .collect();

    jinn_picker::make_items_with_hooks(
        entries,
        jinn_picker::PickerItemHooks::new()
            .row(stream_rule_row)
            .search(stream_rule_search_text),
    )
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::single_range_in_vec_init,
        reason = "test code"
    )]
    use super::*;
    use jinn_theme::default_theme;

    fn rule(name: &str, description: &str) -> StreamRuleConfig {
        StreamRuleConfig {
            name: name.to_owned(),
            description: description.to_owned(),
            conditions: vec!["TODO".to_owned()],
            body: "Finish it.".to_owned(),
            ..Default::default()
        }
    }

    #[rstest::rstest]
    fn row_shows_an_armed_rule_with_the_enabled_mark() {
        // Given one armed rule entry.
        let entry = StreamRuleEntry::from_config(&rule("no-todos", "no TODOs"), &default_theme());

        // When rendering its row unselected with no filter matches.
        let line = stream_rule_row(&entry, &RowCtx::flat(false, &Vec::new()));

        // Then the row carries the enabled mark before the name, and the
        // description after an em-dash.
        let text = line.to_string();
        assert!(text.contains('\u{2713}'), "armed rule must show ✓: {text}");
        assert!(text.contains("no-todos"), "got: {text}");
        assert!(text.contains("no TODOs"), "got: {text}");
    }

    #[rstest::rstest]
    fn row_shows_a_disabled_rule_with_the_disabled_mark() {
        // Given one switched-off rule entry.
        let mut config = rule("no-todos", "no TODOs");
        config.disabled = true;
        let entry = StreamRuleEntry::from_config(&config, &default_theme());

        // When rendering its row.
        let line = stream_rule_row(&entry, &RowCtx::flat(false, &Vec::new()));

        // Then the row carries the disabled mark, not the enabled one.
        let text = line.to_string();
        assert!(text.contains('\u{2717}'), "off rule must show ✗: {text}");
        assert!(!text.contains('\u{2713}'), "got: {text}");
    }

    #[rstest::rstest]
    fn from_config_carries_the_rule_name_description_and_state() {
        // Given a configured rule that is disabled.
        let mut config = rule("no-todos", "no TODOs");
        config.disabled = true;

        // When building the picker's entry from it.
        let entry = StreamRuleEntry::from_config(&config, &default_theme());

        // Then the entry carries everything the row and the confirm need.
        assert_eq!(entry.name, "no-todos");
        assert_eq!(entry.description, "no TODOs");
        assert!(entry.disabled);
    }

    #[rstest::rstest]
    fn build_entries_keeps_the_configuration_order() {
        // Given configured rules in a non-alphabetical order.
        let rules = vec![rule("zeta", "last"), rule("alpha", "first")];

        // When building the picker's entries.
        let items = build_stream_rule_entries(&rules, &default_theme());

        // Then they stay in configuration order — the order the matcher
        // fires in, which is the order the list should read in.
        let names: Vec<&str> = items
            .iter()
            .map(jinn_selection_widget::TreeItem::id)
            .collect();
        assert_eq!(names, ["zeta", "alpha"]);
    }

    #[rstest::rstest]
    fn search_text_matches_on_name_or_description() {
        // Given one entry.
        let entry = StreamRuleEntry::from_config(&rule("no-todos", "no TODOs"), &default_theme());

        // When building its search text.
        let text = stream_rule_search_text(&entry);

        // Then it contains both the name and the description.
        assert_eq!(text, "no-todos no TODOs");
    }

    #[rstest::rstest]
    fn reset_clears_the_filter_the_highlight_and_the_status() {
        // Given a picker state with a filter, a moved highlight, and a
        // status line left by an earlier confirm.
        let mut state = StreamRulePickerState::default();
        state.selection.set_items(build_stream_rule_entries(
            &[rule("a", "x"), rule("b", "y")],
            &default_theme(),
        ));
        state.selection.insert_char('a');
        state.selection.move_down(state.results_viewport);
        state.status = Some("disabled 'a'".to_owned());

        // When resetting for a fresh open.
        state.reset();

        // Then the filter is empty, the highlight is back at the top, and
        // the stale status is gone — a fresh open never inherits the last
        // one's narrowing or its feedback.
        assert!(state.selection.filter().is_empty());
        assert_eq!(
            state
                .selection
                .selected_item()
                .map(|i| i.entry().name.clone())
                .expect("reset picker still has rows"),
            "a"
        );
        assert_eq!(state.status, None);
    }
}
