// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Judge picker entry - display type for the selection widget.

use std::ops::Range;

use jinn_selection_widget::PickerItem;
use jinn_selection_widget::highlight_text_with_bg;
use ratatui::text::{Line, Span};

use crate::feat::judge::Judge;
use crate::feat::theme::Theme;

use crate::feat::picker::style::{active_marker, dim_style, selected_style};

/// A judge definition displayed in the picker.
#[derive(Debug, Clone)]
pub struct JudgePickerEntry {
    /// Judge name (unique identifier).
    pub name: String,
    /// Short description for the picker display.
    pub description: String,
    /// Whether a judge session with this name is already attached to the origin.
    pub already_attached: bool,
    /// Theme for rendering.
    pub theme: Theme,
}

impl JudgePickerEntry {
    /// Create a picker entry from a `Judge` definition.
    #[must_use]
    pub fn from_judge(judge: &Judge, already_attached: bool, theme: Theme) -> Self {
        Self {
            name: judge.name.clone(),
            description: judge.description.clone(),
            already_attached,
            theme,
        }
    }
}

impl PickerItem for JudgePickerEntry {
    fn display_label(&self) -> &str {
        &self.name
    }

    fn render_row(&self, is_selected: bool) -> Line<'static> {
        render_judge_row(
            &self.name,
            &self.description,
            self.already_attached,
            is_selected,
            &[],
            &self.theme,
        )
    }

    fn render_row_with_highlight(
        &self,
        is_selected: bool,
        match_indices: &[Range<usize>],
    ) -> Line<'static> {
        render_judge_row(
            &self.name,
            &self.description,
            self.already_attached,
            is_selected,
            match_indices,
            &self.theme,
        )
    }
}

/// Renders a judge picker row.
fn render_judge_row(
    name: &str,
    description: &str,
    already_attached: bool,
    is_selected: bool,
    match_indices: &[Range<usize>],
    theme: &Theme,
) -> Line<'static> {
    let mut spans = vec![active_marker(is_selected, theme)];

    let base_style = selected_style(is_selected, theme);
    let desc_style = dim_style(is_selected, theme);

    if match_indices.is_empty() {
        spans.push(Span::styled(name.to_owned(), base_style));
    } else {
        spans.extend(highlight_text_with_bg(
            name,
            base_style,
            match_indices,
            theme.picker_highlight_bg,
        ));
    }

    if already_attached {
        spans.push(Span::styled("  (attached)", desc_style));
    }

    if !description.is_empty() {
        spans.push(Span::styled(format!(" - {description}"), desc_style));
    }

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::feat::theme::default_theme;

    fn entry(name: &str, description: &str, already_attached: bool) -> JudgePickerEntry {
        JudgePickerEntry {
            name: name.to_owned(),
            description: description.to_owned(),
            already_attached,
            theme: default_theme(),
        }
    }

    #[rstest::rstest]
    fn display_label_returns_name() {
        let e = entry("accuracy", "Checks accuracy", false);
        assert_eq!(e.display_label(), "accuracy");
    }

    #[rstest::rstest]
    fn render_row_unselected_has_spaces() {
        let e = entry("accuracy", "", false);
        let line = e.render_row(false);
        let text = line.to_string();
        assert!(text.starts_with("  accuracy"));
    }

    #[rstest::rstest]
    fn render_row_selected_has_arrow() {
        let e = entry("accuracy", "", false);
        let line = e.render_row(true);
        let text = line.to_string();
        assert!(text.starts_with("> accuracy"));
    }

    #[rstest::rstest]
    fn render_row_shows_attached_indicator() {
        let e = entry("accuracy", "", true);
        let line = e.render_row(false);
        let text = line.to_string();
        assert!(text.contains("(attached)"));
    }

    #[rstest::rstest]
    fn render_row_shows_description() {
        let e = entry("accuracy", "Checks accuracy", false);
        let line = e.render_row(false);
        let text = line.to_string();
        assert!(text.contains("Checks accuracy"));
    }
}
