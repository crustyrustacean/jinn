//! [`PersonaSection`] — the persona sidebar section.
//!
//! Implements [`SidebarSection`] for displaying the active persona.
//! Shows a header line and a single selectable entry with the persona name.
//! Pressing `e` while this section is focused opens the persona picker.

use crate::common::app_state::AppState;
use crate::feat::ui::sidebar::section_trait::{
    SidebarIntent, SidebarSection, SidebarSectionConfig, SidebarSectionId, SidebarSectionResult,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// Solid full block used as the selection indicator (same as pins section).
const SELECTED_INDICATOR: &str = "\u{2588}";
/// One space used as the unselected border (same as pins section).
const UNSELECTED_BORDER: &str = " ";

/// The persona sidebar section.
///
/// Renders the active persona as a single selectable entry.
/// Always reports `UnhandledDown`/`UnhandledUp` since there is only one item.
#[derive(Debug)]
pub struct PersonaSection;

impl SidebarSection for PersonaSection {
    fn id(&self) -> SidebarSectionId {
        SidebarSectionId::Persona
    }

    fn handle_intent(
        &mut self,
        intent: &SidebarIntent,
        _state: &mut AppState,
        _config: &SidebarSectionConfig,
    ) -> SidebarSectionResult {
        match intent {
            // Single item — always at bottom boundary.
            SidebarIntent::MoveDown => SidebarSectionResult::UnhandledDown,
            // Single item — always at top boundary.
            SidebarIntent::MoveUp => SidebarSectionResult::UnhandledUp,
            SidebarIntent::Action(_) => SidebarSectionResult::Handled,
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        let is_focused = state.frontend.sidebar.focused_section == SidebarSectionId::Persona;
        let sidebar_focused = state.frontend.scope_stack.is_sidebar();
        let theme = &state.frontend.theme;

        let indicator_color = if sidebar_focused {
            theme.focus_accent
        } else {
            theme.border_unfocused
        };

        let is_selected = is_focused;
        let indicator = if is_selected {
            Span::styled(SELECTED_INDICATOR, Style::default().fg(indicator_color))
        } else {
            Span::raw(UNSELECTED_BORDER)
        };

        let persona_name = state
            .context
            .active_persona
            .as_ref()
            .map_or("none", |p| p.name.as_str());

        let lines = {
            let mut lines = Vec::new();
            // Header.
            lines.push(Line::from(vec![Span::styled(
                " Persona",
                Style::default()
                    .fg(theme.primary_text)
                    .add_modifier(Modifier::BOLD),
            )]));
            // Blank separator.
            lines.push(Line::from(""));
            // Entry line.
            let name_style = if is_selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            lines.push(Line::from(vec![
                indicator,
                Span::styled(format!(" {persona_name}"), name_style),
            ]));
            lines
        };

        let widget = Paragraph::new(lines).block(Block::default().borders(Borders::NONE));
        frame.render_widget(widget, area);
    }

    fn content_height(&self, _state: &AppState) -> u16 {
        // Header(1) + blank(1) + entry(1) + trailing gap(1) = 4.
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::app_state::AppState;
    use crate::feat::persona::Persona;
    use crate::feat::ui::sidebar::section_trait::{
        SidebarSectionConfig, SidebarSectionId, SidebarSectionResult,
    };

    fn config_isolated() -> SidebarSectionConfig {
        SidebarSectionConfig {
            has_above: false,
            has_below: false,
        }
    }

    // --- Section identity ---

    #[rstest::rstest]
    fn section_id_is_persona() {
        // Given a PersonaSection.
        let section = PersonaSection;

        // When asking for its ID.
        // Then it returns Persona.
        assert_eq!(section.id(), SidebarSectionId::Persona);
    }

    // --- Content height ---

    #[rstest::rstest]
    fn content_height_is_four_with_active_persona() {
        // Given a PersonaSection and state with an active persona.
        let section = PersonaSection;
        let mut state = AppState::default();
        state.context.active_persona = Some(Persona {
            name: "coding-assistant".to_owned(),
            description: "Expert coder".to_owned(),
            body: String::new(),
            file_path: std::path::PathBuf::from("test.md"),
        });

        // When asking for content height.
        let height = section.content_height(&state);

        // Then it returns 4 (header + blank + entry + trailing gap).
        assert_eq!(height, 4);
    }

    #[rstest::rstest]
    fn content_height_is_four_without_persona() {
        // Given a PersonaSection and state with no active persona.
        let section = PersonaSection;
        let state = AppState::default();

        // When asking for content height.
        let height = section.content_height(&state);

        // Then it returns 4 (consistent layout).
        assert_eq!(height, 4);
    }

    // --- Navigation ---

    #[rstest::rstest]
    fn move_down_returns_unhandled_down() {
        // Given a PersonaSection.
        let mut section = PersonaSection;
        let mut state = AppState::default();

        // When handling MoveDown.
        let result = section.handle_intent(
            &SidebarIntent::MoveDown,
            &mut state,
            &config_isolated(),
        );

        // Then it returns UnhandledDown.
        assert_eq!(result, SidebarSectionResult::UnhandledDown);
    }

    #[rstest::rstest]
    fn move_up_returns_unhandled_up() {
        // Given a PersonaSection.
        let mut section = PersonaSection;
        let mut state = AppState::default();

        // When handling MoveUp.
        let result =
            section.handle_intent(&SidebarIntent::MoveUp, &mut state, &config_isolated());

        // Then it returns UnhandledUp.
        assert_eq!(result, SidebarSectionResult::UnhandledUp);
    }

    #[rstest::rstest]
    fn action_returns_handled() {
        // Given a PersonaSection.
        let mut section = PersonaSection;
        let mut state = AppState::default();

        // When handling an Action intent.
        let result = section.handle_intent(
            &SidebarIntent::Action(crate::Intent::Quit),
            &mut state,
            &config_isolated(),
        );

        // Then it returns Handled.
        assert_eq!(result, SidebarSectionResult::Handled);
    }

    // --- Rendering ---

    use nullslop_testutil::setup_term;

    fn render_rows(
        section: &mut PersonaSection,
        state: &AppState,
        width: u16,
        height: u16,
    ) -> Vec<String> {
        let (mut terminal, area) = setup_term(width, height);
        terminal
            .draw(|frame| {
                section.render(frame, area, state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| {
                        buffer
                            .cell((x, y))
                            .map_or(" ", ratatui::buffer::Cell::symbol)
                    })
                    .collect()
            })
            .collect()
    }

    #[rstest::rstest]
    fn render_shows_persona_header() {
        // Given a PersonaSection.
        let mut section = PersonaSection;
        let state = AppState::default();

        // When rendering.
        let rows = render_rows(&mut section, &state, 30, 5);

        // Then the first row contains "Persona".
        assert!(rows[0].contains("Persona"));
    }

    #[rstest::rstest]
    fn render_shows_persona_name_when_active() {
        // Given a PersonaSection with an active persona.
        let mut section = PersonaSection;
        let mut state = AppState::default();
        state.context.active_persona = Some(Persona {
            name: "coding-assistant".to_owned(),
            description: "Expert coder".to_owned(),
            body: String::new(),
            file_path: std::path::PathBuf::from("test.md"),
        });

        // When rendering.
        let rows = render_rows(&mut section, &state, 40, 5);

        // Then the entry row contains the persona name.
        let combined = rows.join("\n");
        assert!(
            combined.contains("coding-assistant"),
            "should contain persona name, got: {combined}"
        );
    }

    #[rstest::rstest]
    fn render_shows_none_when_no_persona() {
        // Given a PersonaSection with no active persona.
        let mut section = PersonaSection;
        let state = AppState::default();

        // When rendering.
        let rows = render_rows(&mut section, &state, 40, 5);

        // Then the entry row contains "none".
        let combined = rows.join("\n");
        assert!(
            combined.contains("none"),
            "should contain 'none', got: {combined}"
        );
    }
}
