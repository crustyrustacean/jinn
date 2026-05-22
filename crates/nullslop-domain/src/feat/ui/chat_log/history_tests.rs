#![allow(clippy::expect_used, clippy::indexing_slicing)]

use crate::common::app_state::{AppState, FocusScope};
use crate::common::ui_element::UiElement;
use crate::feat::session::tool_result_status::ToolResultStatus;
use crate::feat::ui::chat_log::history::ChatLogElement;
use crate::feat::ui::chat_log::shared::GUTTER_WIDTH;
use crate::protocol::{ChatEntry, PinPosition};
use nullslop_testutil::setup_term;
use ratatui::style::Color;

// --- Gutter width constant for test offsets ---
const G: u16 = GUTTER_WIDTH; // = 2

#[rstest::rstest]
fn name_returns_chat_log() {
    // Given a ChatLogElement.
    let element = ChatLogElement::new();

    // When querying the name.
    let name = element.name();

    // Then it is "chat-log".
    assert_eq!(name, "chat-log");
}

#[rstest::rstest]
fn render_few_messages_bottom_aligned() {
    // Given a ChatLogElement with one user entry in a 40x10 viewport.
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        s.active_session_mut().push_entry(ChatEntry::user("hello"));
        s
    };

    let (mut terminal, area) = setup_term(40, 10);

    // When rendering.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the user text appears in the content area (above the bottom padding).
    let buffer = terminal.backend().buffer().clone();
    let content_cell = buffer.cell((G, 8)).expect("cell should exist");
    assert_eq!(content_cell.symbol(), "h");
}

#[rstest::rstest]
fn chat_log_element_is_selectable() {
    // Given a ChatLogElement.
    let element = ChatLogElement::new();

    // When calling is_selectable.
    let selectable: &dyn UiElement<AppState> = &element;

    // Then it returns true.
    assert!(selectable.is_selectable());
}

#[rstest::rstest]
fn selected_entry_gutter_is_yellow() {
    // Given a ChatLogElement with 2 entries, first selected.
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        s.active_session_mut().push_entry(ChatEntry::user("hello"));
        s.active_session_mut().push_entry(ChatEntry::user("world"));
        // push_entry auto-selects last (index 1). Move to index 0.
        s.active_session_mut().select_prev_entry();
        s
    };

    let (mut terminal, area) = setup_term(40, 10);

    // When rendering.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the selected entry's gutter has yellow fg.
    // 2 entries × 3 lines = 6, 4 blank above. Entry 0 at rows 4-6.
    let buffer = terminal.backend().buffer().clone();
    let gutter_cell = buffer.cell((0, 5)).expect("cell should exist");
    assert_eq!(gutter_cell.style().fg, Some(Color::Yellow));

    // And the unselected entry's gutter has the context included color (not ignored).
    let unselected_gutter = buffer.cell((0, 8)).expect("cell should exist");
    assert_eq!(
        unselected_gutter.style().fg,
        Some(crate::feat::theme::default_theme().gutter_context_included)
    );
}

#[rstest::rstest]
fn selected_entry_gutter_is_dark_gray_when_unfocused() {
    // Given a ChatLogElement with a selected entry, sidebar focused.
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        s.active_session_mut().push_entry(ChatEntry::user("hello"));
        s.active_session_mut().push_entry(ChatEntry::user("world"));
        s.active_session_mut().select_prev_entry(); // index 0
        s.frontend.scope_stack.push(FocusScope::SidebarPersona);
        s
    };

    let (mut terminal, area) = setup_term(40, 10);

    // When rendering.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the selected entry's gutter has dark gray fg (inactive border color, not yellow).
    // 2 entries × 3 lines = 6, 4 blank above. Entry 0 content at row 5.
    let buffer = terminal.backend().buffer().clone();
    let gutter_cell = buffer.cell((0, 5)).expect("cell should exist");
    assert_eq!(
        gutter_cell.style().fg,
        Some(crate::feat::theme::default_theme().border_unfocused)
    );
}

#[rstest::rstest]
fn selected_entry_gutter_is_dark_gray_when_input_focused() {
    // Given a ChatLogElement with a selected entry, input focused.
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        s.active_session_mut().push_entry(ChatEntry::user("hello"));
        s.active_session_mut().push_entry(ChatEntry::user("world"));
        s.active_session_mut().select_prev_entry(); // index 0
        s.frontend.scope_stack.push(FocusScope::Input);
        s
    };

    let (mut terminal, area) = setup_term(40, 10);

    // When rendering.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the selected entry's gutter has dark gray fg (inactive border color).
    // 2 entries × 3 lines = 6, 4 blank above. Entry 0 content at row 5.
    let buffer = terminal.backend().buffer().clone();
    let gutter_cell = buffer.cell((0, 5)).expect("cell should exist");
    assert_eq!(
        gutter_cell.style().fg,
        Some(crate::feat::theme::default_theme().border_unfocused)
    );
}

#[rstest::rstest]
fn render_stores_viewport_state() {
    // Given a ChatLogElement with entries.
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        s.active_session_mut().push_entry(ChatEntry::user("hello"));
        s.active_session_mut().push_entry(ChatEntry::user("world"));
        s
    };

    let (mut terminal, area) = setup_term(40, 10);

    // When rendering.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then viewport state is stored in the session.
    let range = state.active_session().visible_entry_range();
    assert!(
        !range.is_empty(),
        "entry_line_ranges should be populated after render"
    );
}

#[rstest::rstest]
fn render_pinned_entry_shows_pin_in_gutter() {
    // Given a ChatLogElement with one pinned user entry.
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        s.active_session_mut()
            .push_entry(ChatEntry::user("hello").with_pin(PinPosition::Top));
        s
    };

    let (mut terminal, area) = setup_term(40, 10);

    // When rendering.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the gutter contains the 📌 character.
    let buffer = terminal.backend().buffer().clone();
    let has_pin = (0..10).any(|row| {
        (0..2).any(|col| {
            buffer
                .cell((col, row))
                .is_some_and(|c| c.symbol() == "\u{1F4CC}")
        })
    });
    assert!(
        has_pin,
        "pinned entry should show \u{1F4CC} pin icon in gutter"
    );
}

#[rstest::rstest]
fn render_unpinned_entry_has_no_pin_icon() {
    // Given a ChatLogElement with one unpinned user entry.
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        s.active_session_mut().push_entry(ChatEntry::user("hello"));
        s
    };

    let (mut terminal, area) = setup_term(40, 10);

    // When rendering.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then no cell in the buffer contains the 📌 character.
    let buffer = terminal.backend().buffer().clone();
    let has_pin = (0..10).any(|row| {
        (0..40).any(|col| {
            buffer
                .cell((col, row))
                .is_some_and(|c| c.symbol() == "\u{1F4CC}")
        })
    });
    assert!(
        !has_pin,
        "unpinned entry should not show \u{1F4CC} pin icon"
    );
}

#[rstest::rstest]
fn render_pinned_multi_line_entry_shows_exactly_one_pin() {
    // Given a ChatLogElement with one pinned multi-line user entry.
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        s.active_session_mut().push_entry(
            ChatEntry::user("line one\nline two\nline three").with_pin(PinPosition::Top),
        );
        s
    };

    let (mut terminal, area) = setup_term(40, 10);

    // When rendering.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then exactly one pin icon appears in the gutter.
    let buffer = terminal.backend().buffer().clone();
    let pin_count = (0..10)
        .filter(|&row| {
            (0..2).any(|col| {
                buffer
                    .cell((col, row))
                    .is_some_and(|c| c.symbol() == "\u{1F4CC}")
            })
        })
        .count();
    assert_eq!(
        pin_count, 1,
        "multi-line pinned entry should show exactly one pin icon, found {pin_count}"
    );
}

#[rstest::rstest]
fn render_scroll_to_selected_keeps_entry_visible() {
    // Given a ChatLogElement with many entries where the first is selected
    // and the viewport is small enough that it would normally be scrolled off.
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        // Add 20 entries (each 1 line).
        for i in 0..20 {
            s.active_session_mut()
                .push_entry(ChatEntry::user(format!("msg {i}")));
        }
        // Select the first entry (index 0).
        s.active_session_mut().select_next_entry(); // selects index 0
        s
    };

    let (mut terminal, area) = setup_term(40, 5); // 5-line viewport

    // When rendering.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the selected entry's gutter (yellow fg) should be visible in the viewport.
    let buffer = terminal.backend().buffer().clone();
    let has_yellow_gutter = (0..5).any(|row| {
        buffer
            .cell((0, row))
            .is_some_and(|c| c.style().fg == Some(Color::Yellow))
    });
    assert!(
        has_yellow_gutter,
        "selected entry should be visible in viewport when scroll-to-selected is active"
    );
}

#[rstest::rstest]
fn render_thinking_entry_appears_above_assistant() {
    // Given a ChatLogElement with thinking then assistant entries.
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        s.active_session_mut()
            .push_entry(ChatEntry::thinking("reasoning"));
        s.active_session_mut()
            .push_entry(ChatEntry::assistant("response"));
        s
    };

    let (mut terminal, area) = setup_term(40, 10);

    // When rendering.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the thinking entry appears above the assistant entry.
    // Thinking = 2 lines (pad + content), assistant = 3 lines (pad + content + pad).
    // Total = 5, 5 blank above. Thinking content at row 6, assistant content at row 8.
    let buffer = terminal.backend().buffer().clone();
    // Row 6 has the thinking content ("reasoning").
    let thinking_cell = buffer.cell((G, 6)).expect("cell should exist");
    assert_eq!(thinking_cell.symbol(), "r");
    // Row 8 has the assistant content ("response").
    let assistant_cell = buffer.cell((G, 8)).expect("cell should exist");
    assert_eq!(assistant_cell.symbol(), "r");
}

#[rstest::rstest]
fn render_pinned_selected_entry_gutter_has_focus_accent_bg() {
    // Given a ChatLogElement with one pinned user entry (auto-selected).
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        s.active_session_mut()
            .push_entry(ChatEntry::user("hello").with_pin(PinPosition::Top));
        s
    };

    let (mut terminal, area) = setup_term(40, 10);

    // When rendering.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the pinned entry's gutter has focus_accent (yellow) background.
    // Entry is 3 lines (pad + content + pad), starts at row 7 in 10-line viewport.
    // The pin icon and yellow bg appear on the first line of the entry (row 7).
    let buffer = terminal.backend().buffer().clone();
    let gutter_cell = buffer.cell((0, 7)).expect("cell should exist");
    assert_eq!(
        gutter_cell.style().bg,
        Some(Color::Yellow),
        "pinned selected entry gutter should have yellow background"
    );
}

#[rstest::rstest]
fn render_pinned_unselected_entry_gutter_has_default_bg() {
    // Given a ChatLogElement with a pinned entry and an unpinned entry (unpinned selected).
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        s.active_session_mut()
            .push_entry(ChatEntry::user("pinned").with_pin(PinPosition::Top));
        s.active_session_mut()
            .push_entry(ChatEntry::user("unpinned"));
        // push_entry auto-selects last (index 1, unpinned).
        s
    };

    let (mut terminal, area) = setup_term(40, 10);

    // When rendering.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the pinned (unselected) entry's gutter has no yellow background.
    let buffer = terminal.backend().buffer().clone();
    let gutter_cell = buffer.cell((0, 8)).expect("cell should exist");
    assert_ne!(
        gutter_cell.style().bg,
        Some(Color::Yellow),
        "pinned unselected entry gutter should not have yellow background"
    );
}

#[rstest::rstest]
fn render_unpinned_selected_entry_gutter_has_no_focus_accent_bg() {
    // Given a ChatLogElement with one unpinned user entry (auto-selected).
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        s.active_session_mut().push_entry(ChatEntry::user("hello"));
        s
    };

    let (mut terminal, area) = setup_term(40, 10);

    // When rendering.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the unpinned selected entry's gutter has no yellow background.
    let buffer = terminal.backend().buffer().clone();
    let gutter_cell = buffer.cell((0, 9)).expect("cell should exist");
    assert_ne!(
        gutter_cell.style().bg,
        Some(Color::Yellow),
        "unpinned selected entry gutter should not have yellow background"
    );
}

#[rstest::rstest]
fn render_pinned_selected_unfocused_entry_gutter_has_border_unfocused_bg() {
    // Given a ChatLogElement with one pinned entry selected, sidebar focused.
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        s.active_session_mut()
            .push_entry(ChatEntry::user("hello").with_pin(PinPosition::Top));
        s.frontend.scope_stack.push(FocusScope::SidebarPersona);
        s
    };

    let (mut terminal, area) = setup_term(40, 10);

    // When rendering.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the pinned entry's gutter has border_unfocused background, not yellow.
    // 1 entry × 3 lines = 3, 7 blank above. Entry at rows 7-9, pin icon at row 7.
    let buffer = terminal.backend().buffer().clone();
    let gutter_cell = buffer.cell((0, 7)).expect("cell should exist");
    assert_eq!(
        gutter_cell.style().bg,
        Some(crate::feat::theme::default_theme().border_unfocused),
        "pinned selected unfocused entry gutter should have border_unfocused background"
    );
}

#[rstest::rstest]
fn render_long_session_shows_last_entry_at_bottom() {
    // Given a ChatLogElement with many assistant entries containing word-wrapping text.
    // Assistant entries are not padded, so they wrap at word boundaries.
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        for i in 0..20 {
            s.active_session_mut()
                .push_entry(ChatEntry::assistant(format!(
                    "This is message number {i} with some long words that will wrap"
                )));
        }
        s
    };

    // 30-wide, 10-tall viewport (content width = 28 after 2-char gutter).
    let (mut terminal, area) = setup_term(30, 10);

    // When rendering at bottom (auto-scroll).
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the last entry's text appears near the bottom of the buffer.
    // Each entry is 3 lines (pad + content + pad), so the bottom row is padding.
    // Check rows 8-9 for the content or padding.
    let buffer = terminal.backend().buffer().clone();
    let has_last_entry = (7..10).any(|row| {
        let row_text: String = (0..30)
            .filter_map(|x| buffer.cell((x, row)).map(|c| c.symbol().to_owned()))
            .collect();
        row_text.contains("wrap") || row_text.contains("will") || row_text.contains("19")
    });
    assert!(
        has_last_entry,
        "last entry's text should be visible near the bottom of the viewport"
    );
}

#[rstest::rstest]
fn render_scroll_to_bottom_shows_full_last_entry() {
    // Given a ChatLogElement with assistant entries containing word-wrapping text.
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        for i in 0..15 {
            s.active_session_mut()
                .push_entry(ChatEntry::assistant(format!(
                    "This is message number {i} with some long words that will wrap"
                )));
        }
        // Simulate pressing G: scroll to bottom + select last entry.
        s.active_session_mut().scroll_to_bottom();
        let max = s.active_session().history().len() - 1;
        s.active_session_mut().set_selected_entry_index(max);
        s
    };

    let (mut terminal, area) = setup_term(30, 10);

    // When rendering.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the last entry's content ("message number 14") is visible in the viewport.
    let buffer = terminal.backend().buffer().clone();
    let has_last_entry = (0..10).any(|row| {
        let row_text: String = (0..30)
            .filter_map(|x| buffer.cell((x, row)).map(|c| c.symbol().to_owned()))
            .collect();
        row_text.contains("14")
    });
    assert!(
        has_last_entry,
        "last entry (message number 14) should be visible after scroll to bottom"
    );
}

#[rstest::rstest]
fn render_scroll_to_selected_middle_entry_adjusts_viewport() {
    // Given a ChatLogElement with many entries where a middle entry is selected.
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        // 30 entries, each with word-wrapping text.
        for i in 0..30 {
            s.active_session_mut()
                .push_entry(ChatEntry::assistant(format!(
                    "This is message number {i} with some long words that will wrap"
                )));
        }
        // Select entry 10 (middle of 30).
        s.active_session_mut().set_selected_entry_index(10);
        s
    };

    let (mut terminal, area) = setup_term(30, 10);

    // When rendering.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the selected entry is visible (yellow gutter in viewport).
    let buffer = terminal.backend().buffer().clone();
    let has_yellow_gutter = (0..10).any(|row| {
        buffer
            .cell((0, row))
            .is_some_and(|c| c.style().fg == Some(Color::Yellow))
    });
    assert!(
        has_yellow_gutter,
        "selected middle entry should be visible in viewport after scroll-to-selected"
    );

    // And the selected entry's text ("message number 10") is visible.
    let has_entry_10 = (0..10).any(|row| {
        let row_text: String = (0..30)
            .filter_map(|x| buffer.cell((x, row)).map(|c| c.symbol().to_owned()))
            .collect();
        row_text.contains("10")
    });
    assert!(
        has_entry_10,
        "selected middle entry's text should be visible in viewport"
    );
}

#[rstest::rstest]
fn render_scroll_down_through_tall_entry_works() {
    // Given a tall entry (50 lines) in a small (10-line) viewport, scrolled to show
    // the middle of the entry.
    let mut element = ChatLogElement::new();
    let mut state = AppState::default();
    let long_text: String = (0..50)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    state
        .active_session_mut()
        .push_entry(ChatEntry::assistant(long_text));
    // First render to populate last_max_offset, then scroll.
    let (mut terminal, area) = setup_term(40, 10);
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();
    // Now scroll up to show the middle of the tall entry.
    state.active_session_mut().scroll_up(20);
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the viewport shows text from the middle of the entry, not the top.
    let buffer = terminal.backend().buffer().clone();
    let viewport_text: String = (0..10)
        .map(|row| {
            (0..40)
                .filter_map(|col| buffer.cell((col, row)).map(|c| c.symbol().to_owned()))
                .collect::<String>()
        })
        .collect();
    assert!(
        !viewport_text.contains("line 0"),
        "viewport should not show line 0 when scrolled to middle, got: {viewport_text}"
    );
}

#[rstest::rstest]
fn render_tall_entry_snaps_when_completely_below_viewport() {
    // Given a tall entry at the end and the viewport scrolled to the top,
    // with the tall entry selected.
    let mut element = ChatLogElement::new();
    let mut state = AppState::default();
    // Push 20 short entries to fill space.
    for i in 0..20 {
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant(format!("msg {i}")));
    }
    // Push a tall entry (50 lines).
    let long_text: String = (0..50)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    state
        .active_session_mut()
        .push_entry(ChatEntry::assistant(long_text));
    // push_entry auto-selects last entry (the tall one).

    let (mut terminal, area) = setup_term(40, 5);

    // First render to populate last_max_offset.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Scroll to top so the tall entry is completely below the viewport.
    state.active_session_mut().scroll_to_top();
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the renderer snaps to show the tall entry's start.
    let buffer = terminal.backend().buffer().clone();
    let viewport_text: String = (0..5)
        .map(|row| {
            (0..40)
                .filter_map(|col| buffer.cell((col, row)).map(|c| c.symbol().to_owned()))
                .collect::<String>()
        })
        .collect();
    assert!(
        viewport_text.contains("line 0"),
        "tall entry below viewport should snap to show its start, got: {viewport_text}"
    );
}

#[rstest::rstest]
fn virtualization_populates_cache_after_render() {
    // Given a ChatLogElement with many entries.
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        for i in 0..30 {
            s.active_session_mut()
                .push_entry(ChatEntry::assistant(format!("msg {i}")));
        }
        s
    };

    let (mut terminal, area) = setup_term(40, 10);

    // When rendering.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the cache has entries for all 30 entries.
    assert_eq!(
        element.line_cache.len(),
        30,
        "cache should have entries for all 30 entries after render"
    );
}

#[rstest::rstest]
fn expand_collapse_invalidates_and_rerenders() {
    // Given a ChatLogElement with a long tool result entry.
    let mut element = ChatLogElement::new();
    let long_content: String = (0..20)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let entry = ChatEntry::tool_result("call1", "bash", &long_content, ToolResultStatus::Success);
    let entry_id = entry.id.clone();
    let mut state = AppState::default();
    state.active_session_mut().push_entry(entry);

    let (mut terminal, area) = setup_term(80, 30);

    // When rendering (truncated — max_lines=5 by default).
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the truncation indicator is visible in the buffer.
    let buffer = terminal.backend().buffer().clone();
    let has_more_lines = (0..30).any(|row| {
        let row_text: String = (2..80)
            .filter_map(|col| buffer.cell((col, row)).map(|c| c.symbol().to_owned()))
            .collect();
        row_text.contains("lines hidden above")
    });
    assert!(
        has_more_lines,
        "truncated tool result should show truncation indicator"
    );

    // When expanding the entry and re-rendering.
    state.active_session_mut().toggle_expand_entry(entry_id);
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the expanded content shows all lines.
    let buffer2 = terminal.backend().buffer().clone();
    let has_line_19 = (0..30).any(|row| {
        let row_text: String = (2..80)
            .filter_map(|col| buffer2.cell((col, row)).map(|c| c.symbol().to_owned()))
            .collect();
        row_text.contains("line 19")
    });
    assert!(
        has_line_19,
        "expanded tool result should show all content including line 19"
    );
}

#[rstest::rstest]
fn resize_clears_cache_and_rerenders() {
    // Given a ChatLogElement rendered at width 40.
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        for i in 0..5 {
            s.active_session_mut()
                .push_entry(ChatEntry::assistant(format!("message {i}")));
        }
        s
    };

    let (mut terminal, area) = setup_term(40, 10);
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // When rendering at a different width (simulating resize).
    let (mut terminal2, area2) = setup_term(60, 10);
    terminal2
        .draw(|frame| {
            element.render(frame, area2, &state);
        })
        .unwrap();

    // Then the cache is still populated (re-populated at new width).
    assert_eq!(
        element.line_cache.len(),
        5,
        "cache should be re-populated after resize"
    );

    // And the last message is visible near the bottom.
    let buffer = terminal2.backend().buffer().clone();
    let has_last_message = (7..10).any(|row| {
        let row_text: String = (0..60)
            .filter_map(|x| buffer.cell((x, row)).map(|c| c.symbol().to_owned()))
            .collect();
        row_text.contains('4')
    });
    assert!(
        has_last_message,
        "last message should be visible after resize"
    );
}

#[rstest::rstest]
fn streaming_content_change_invalidates_cache() {
    // Given a ChatLogElement rendered during active streaming.
    let mut element = ChatLogElement::new();
    let (mut terminal, area) = setup_term(40, 10);

    let mut state = AppState::default();
    state.active_session_mut().begin_streaming();
    state
        .active_session_mut()
        .append_stream_token("initial")
        .expect("ok");

    // When rendering with initial streaming content.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    assert_eq!(element.line_cache.len(), 1, "cache should have 1 entry");

    // When more tokens arrive (content changes, fingerprint changes).
    state
        .active_session_mut()
        .append_stream_token(" + more text")
        .expect("ok");

    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the cache still has 1 entry (re-computed with new fingerprint).
    assert_eq!(
        element.line_cache.len(),
        1,
        "cache should have 1 entry after streaming token append"
    );

    // And the updated content is visible.
    let buffer = terminal.backend().buffer().clone();
    let has_more = (0..10).any(|row| {
        let row_text: String = (2..40)
            .filter_map(|col| buffer.cell((col, row)).map(|c| c.symbol().to_owned()))
            .collect();
        row_text.contains("more")
    });
    assert!(
        has_more,
        "updated content should be visible after streaming"
    );
}

#[rstest::rstest]
fn render_transient_entry_has_muted_text_color() {
    // Given a ChatLogElement with a transient entry.
    let mut element = ChatLogElement::new();
    let state = {
        let mut s = AppState::default();
        s.active_session_mut()
            .push_entry(ChatEntry::transient("Welcome to nullslop!"));
        s
    };

    let (mut terminal, area) = setup_term(40, 10);

    // When rendering.
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the transient text appears with the theme text color.
    let buffer = terminal.backend().buffer().clone();
    let transient_cell = buffer.cell((G, 8)).expect("cell should exist");
    assert_eq!(transient_cell.symbol(), "W");
    assert_eq!(
        transient_cell.fg, state.frontend.theme.primary_text,
        "transient entry should use theme text color (from markdown renderer)"
    );
}
