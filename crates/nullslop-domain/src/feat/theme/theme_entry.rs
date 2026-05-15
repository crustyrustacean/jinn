//! Theme picker entry type and rendering.

use crate::feat::theme::Theme;
use nullslop_selection_widget::PickerItem;
use ratatui::text::Line;

/// A theme entry ready for display in the theme picker.
#[derive(Debug, Clone)]
pub struct ThemeEntry {
    /// Theme name (filename without extension, or "default").
    pub name: String,
    /// The resolved theme colors.
    pub theme: Theme,
}

impl PickerItem for ThemeEntry {
    fn display_label(&self) -> &str {
        &self.name
    }

    fn render_row(&self, is_selected: bool) -> Line<'static> {
        use ratatui::style::Style;
        use ratatui::text::Span;

        let style = if is_selected {
            Style::default()
                .fg(self.theme.primary_text)
                .bg(self.theme.picker_selected_bg)
        } else {
            Style::default()
        };

        // Show a colored accent swatch + name.
        let swatch = Span::styled("\u{2588} ", Style::default().fg(self.theme.focus_accent));
        let name = Span::styled(self.name.clone(), style);
        Line::from(vec![swatch, name])
    }
}
