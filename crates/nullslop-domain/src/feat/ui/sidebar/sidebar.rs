//! [`Sidebar`] — the sidebar container that manages section registration,
//! focus delegation, section-crossing navigation, and rendering.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Block;

use super::section_trait::{
    EnterFrom, SectionNavResult, SidebarIntent, SidebarSection, SidebarSectionId,
};
use super::{persona_section, pins};
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

    /// Renders all sections within the given area.
    ///
    /// Applies a dark gray background to the entire sidebar area,
    /// then renders each section in registration order, stacking vertically.
    /// Sections receive their computed sub-area based on content height.
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        // Clear sidebar area with dark gray background.
        let background =
            Block::default().style(Style::default().bg(state.frontend.theme.gutter_bg));
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
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Sidebar topology — section-crossing navigation
// ---------------------------------------------------------------------------

/// Navigate the sidebar, handling section-crossing when a section exhausts its entries.
///
/// This is the single navigation entry point called by the IntentHandler.
/// Sections report `Exhausted` when they run out of entries; this function
/// decides whether to switch to an adjacent section or keep the cursor where it is.
pub fn navigate_sidebar(direction: SidebarIntent, state: &mut AppState) {
    let focused = state.frontend.sidebar.focused_section;
    let result = dispatch_navigate(focused, &direction, state);

    if result == SectionNavResult::Exhausted {
        let neighbor = match &direction {
            SidebarIntent::MoveDown => next_section(focused),
            SidebarIntent::MoveUp => prev_section(focused),
            SidebarIntent::Action(_) => return,
        };

        if let Some(target) = neighbor
            && section_has_content(target, state)
        {
            clear_cursor(focused, state);
            state.frontend.sidebar.focused_section = target;
            let enter_from = match direction {
                SidebarIntent::MoveDown => EnterFrom::Top,
                SidebarIntent::MoveUp => EnterFrom::Bottom,
                SidebarIntent::Action(_) => return,
            };
            receive_cursor(target, enter_from, state);
        }
    }
}

fn dispatch_navigate(
    section: SidebarSectionId,
    intent: &SidebarIntent,
    state: &mut AppState,
) -> SectionNavResult {
    match section {
        SidebarSectionId::Persona => persona_section::navigate(intent, state),
        SidebarSectionId::Pins => pins::navigate(intent, state),
    }
}

fn next_section(id: SidebarSectionId) -> Option<SidebarSectionId> {
    match id {
        SidebarSectionId::Persona => Some(SidebarSectionId::Pins),
        SidebarSectionId::Pins => None,
    }
}

fn prev_section(id: SidebarSectionId) -> Option<SidebarSectionId> {
    match id {
        SidebarSectionId::Persona => None,
        SidebarSectionId::Pins => Some(SidebarSectionId::Persona),
    }
}

fn section_has_content(id: SidebarSectionId, state: &AppState) -> bool {
    match id {
        SidebarSectionId::Persona => true,
        SidebarSectionId::Pins => !state.sorted_pinned_ids().is_empty(),
    }
}

fn clear_cursor(id: SidebarSectionId, state: &mut AppState) {
    match id {
        SidebarSectionId::Persona => state.frontend.persona_section.cursor = None,
        SidebarSectionId::Pins => state.frontend.pins.clear_selection(),
    }
}

fn receive_cursor(id: SidebarSectionId, enter_from: EnterFrom, state: &mut AppState) {
    match id {
        SidebarSectionId::Persona => persona_section::receive_cursor(state, enter_from),
        SidebarSectionId::Pins => pins::receive_cursor(state, enter_from),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::app_state::FocusScope;
    use crate::feat::ui::sidebar::pins::PinsSection;
    use crate::feat::ui::sidebar::pins::pins_section::handle_sidebar_focus;
    use crate::feat::ui::sidebar::section_trait::SidebarIntent;
    use crate::feat::ui::sidebar::section_trait::SidebarSectionId;
    use crate::protocol::ChatEntry;
    use crate::protocol::PinPosition;

    fn state_with_pinned(count: usize) -> AppState {
        let mut state = AppState::default();
        for i in 0..count {
            let entry = ChatEntry::user(format!("entry {i}"));
            let id = entry.id.clone();
            state.active_session_mut().push_entry(entry);
            state.active_session_mut().pin_entry(&id, PinPosition::Top);
        }
        state
    }

    // --- Registration ---

    #[rstest::rstest]
    fn register_adds_section() {
        // Given a new sidebar.
        let mut sidebar = Sidebar::new();

        // When registering a section.
        sidebar.register(Box::new(PinsSection));

        // Then section count is 1.
        assert_eq!(sidebar.section_count(), 1);
    }

    // --- Rendering ---

    #[rstest::rstest]
    fn render_clears_area_with_sidebar_background() {
        // Given a sidebar with no sections.
        let mut sidebar = Sidebar::new();
        let state = AppState::default();

        let backend = TestBackend::new(30, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        // When rendering.
        terminal
            .draw(|frame| {
                sidebar.render(frame, Rect::new(0, 0, 30, 10), &state);
            })
            .unwrap();

        // Then the entire area has the sidebar background (#191b1e).
        let expected_bg = Color::Rgb(0x19, 0x1b, 0x1e);
        let buf = terminal.backend().buffer();
        for y in 0..10u16 {
            for x in 0..30u16 {
                let cell = buf.cell((x, y)).expect("cell");
                assert_eq!(
                    cell.bg, expected_bg,
                    "cell ({x},{y}) should have #191b1e bg"
                );
            }
        }
    }

    // --- navigate_sidebar ---

    #[rstest::rstest]
    fn move_down_from_persona_with_pins_enters_pins_at_first_entry() {
        // Given persona focused with 3 pinned entries.
        let mut state = state_with_pinned(3);
        state.frontend.sidebar.focused_section = SidebarSectionId::Persona;
        state.frontend.persona_section.cursor = Some(0);

        // When navigating down.
        navigate_sidebar(SidebarIntent::MoveDown, &mut state);

        // Then focus moves to Pins and the first pinned entry is selected.
        assert_eq!(
            state.frontend.sidebar.focused_section,
            SidebarSectionId::Pins
        );
        let first_pin_id = state.sorted_pinned_ids()[0].clone();
        assert_eq!(state.frontend.pins.selected_id(), Some(&first_pin_id));
    }

    #[rstest::rstest]
    fn move_down_from_persona_with_empty_pins_stays_on_persona() {
        // Given persona focused with no pinned entries.
        let mut state = AppState::default();
        state.frontend.sidebar.focused_section = SidebarSectionId::Persona;
        state.frontend.persona_section.cursor = Some(0);

        // When navigating down.
        navigate_sidebar(SidebarIntent::MoveDown, &mut state);

        // Then focus stays on Persona.
        assert_eq!(
            state.frontend.sidebar.focused_section,
            SidebarSectionId::Persona
        );
    }

    #[rstest::rstest]
    fn move_up_from_first_pin_enters_persona() {
        // Given pins focused with 3 entries, first pin selected.
        let mut state = state_with_pinned(3);
        state.frontend.sidebar.focused_section = SidebarSectionId::Pins;
        let first_id = state.sorted_pinned_ids()[0].clone();
        state.frontend.pins.select_by_id(first_id);

        // When navigating up from the first pin.
        navigate_sidebar(SidebarIntent::MoveUp, &mut state);

        // Then focus moves to Persona, pins selection is cleared, and persona has cursor.
        assert_eq!(
            state.frontend.sidebar.focused_section,
            SidebarSectionId::Persona
        );
        assert!(state.frontend.pins.selected_id().is_none());
        assert_eq!(state.frontend.persona_section.cursor, Some(0));
    }

    #[rstest::rstest]
    fn move_down_at_last_pin_sticks() {
        // Given pins focused with 2 entries, last pin selected.
        let mut state = state_with_pinned(2);
        state.frontend.sidebar.focused_section = SidebarSectionId::Pins;
        let last_id = state.sorted_pinned_ids()[1].clone();
        state.frontend.pins.select_by_id(last_id);

        // When navigating down.
        navigate_sidebar(SidebarIntent::MoveDown, &mut state);

        // Then focus stays on Pins.
        assert_eq!(
            state.frontend.sidebar.focused_section,
            SidebarSectionId::Pins
        );
    }

    #[rstest::rstest]
    fn move_up_at_persona_sticks() {
        // Given persona focused.
        let mut state = AppState::default();
        state.frontend.sidebar.focused_section = SidebarSectionId::Persona;
        state.frontend.persona_section.cursor = Some(0);

        // When navigating up.
        navigate_sidebar(SidebarIntent::MoveUp, &mut state);

        // Then focus stays on Persona.
        assert_eq!(
            state.frontend.sidebar.focused_section,
            SidebarSectionId::Persona
        );
    }

    // --- handle_sidebar_focus ---

    #[rstest::rstest]
    fn sidebar_focus_places_cursor_on_persona() {
        // Given default app state.
        let mut state = AppState::default();

        // When handling sidebar focus.
        handle_sidebar_focus(&mut state);

        // Then persona section has the cursor.
        assert_eq!(state.frontend.persona_section.cursor, Some(0));
    }
}
