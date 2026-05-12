//! [`Sidebar`] — the sidebar container that manages section registration,
//! focus delegation, section-crossing navigation, and rendering.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Block;

use super::section_trait::{
    SidebarIntent, SidebarSection, SidebarSectionConfig, SidebarSectionResult,
};
use super::state::SidebarState;
use crate::common::app_state::AppState;

/// The sidebar container.
///
/// Holds registered sections in order, manages focus, and handles
/// section-crossing navigation for `j`/`k` intents.
#[derive(Debug)]
pub struct Sidebar {
    sections: Vec<Box<dyn SidebarSection>>,
}

impl Sidebar {
    /// Creates a new empty sidebar.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sections: Vec::new(),
        }
    }

    /// Registers a section with the sidebar.
    ///
    /// Sections are rendered and navigated in registration order.
    pub fn register(&mut self, section: Box<dyn SidebarSection>) {
        self.sections.push(section);
    }

    /// Returns the number of registered sections.
    #[must_use]
    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// Dispatches a sidebar intent to the focused section.
    ///
    /// If the focused section returns `UnhandledDown` or `UnhandledUp`,
    /// moves focus to the next/previous section and re-dispatches.
    pub fn handle_intent(
        &mut self,
        intent: &SidebarIntent,
        state: &mut AppState,
        sidebar_state: &mut SidebarState,
    ) {
        let Some(focused_index) = self.focused_index(sidebar_state) else {
            return;
        };

        let config = self.config_for(focused_index);
        let section = &mut self.sections[focused_index];
        let result = section.handle_intent(intent, state, &config);

        match result {
            SidebarSectionResult::Handled => {}
            SidebarSectionResult::UnhandledDown => {
                if let Some(next_index) = self.next_section_index(focused_index) {
                    // Move focus down to next section.
                    sidebar_state.focused_section = self.sections[next_index].id();
                    let next_config = self.config_for(next_index);
                    let next_section = &mut self.sections[next_index];
                    // Re-dispatch: entering from above, select first item.
                    next_section.handle_intent(&SidebarIntent::MoveDown, state, &next_config);
                }
                // If no next section, selection sticks (do nothing).
            }
            SidebarSectionResult::UnhandledUp => {
                if let Some(prev_index) = self.prev_section_index(focused_index) {
                    // Move focus up to previous section.
                    sidebar_state.focused_section = self.sections[prev_index].id();
                    let prev_config = self.config_for(prev_index);
                    let prev_section = &mut self.sections[prev_index];
                    // Re-dispatch: entering from below, select last item.
                    prev_section.handle_intent(&SidebarIntent::MoveUp, state, &prev_config);
                }
                // If no prev section, selection sticks (do nothing).
            }
        }
    }

    /// Renders all sections within the given area.
    ///
    /// Applies a dark gray background to the entire sidebar area,
    /// then renders each section in registration order, stacking vertically.
    /// Sections receive their computed sub-area based on content height.
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        // Clear sidebar area with dark gray background.
        let background = Block::default().style(Style::default().bg(Color::DarkGray));
        frame.render_widget(background, area);

        // Stack sections vertically within the sidebar area.
        let mut y_offset = 0u16;
        for section in &mut self.sections {
            let height = section.content_height(state);
            if height == 0 || y_offset >= area.height {
                continue;
            }
            let available = area.height.saturating_sub(y_offset);
            let section_height = height.min(available);
            let section_area = Rect {
                x: area.x,
                y: area.y + y_offset,
                width: area.width,
                height: section_height,
            };
            section.render(frame, section_area, state);
            y_offset += section_height;
        }
    }

    // --- Helpers ---

    fn focused_index(&self, state: &SidebarState) -> Option<usize> {
        self.sections
            .iter()
            .position(|s| s.id() == state.focused_section)
    }

    fn config_for(&self, index: usize) -> SidebarSectionConfig {
        SidebarSectionConfig {
            has_above: index > 0,
            has_below: index < self.sections.len().saturating_sub(1),
        }
    }

    fn next_section_index(&self, current: usize) -> Option<usize> {
        if current + 1 < self.sections.len() {
            Some(current + 1)
        } else {
            None
        }
    }

    fn prev_section_index(&self, current: usize) -> Option<usize> {
        if current > 0 { Some(current - 1) } else { None }
    }
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}
