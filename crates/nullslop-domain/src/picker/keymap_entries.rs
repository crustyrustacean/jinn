//! Keymap entry tests.
//!
//! The [`KeymapEntry`] struct and [`PickerItem`] implementation live
//! in `nullslop-protocol`. This module contains tests for keymap entry behavior.

#[cfg(test)]
mod tests {
    use crate::protocol::KeymapEntry;
    use nullslop_selection_widget::PickerItem;
    use ratatui::style::Color;
    use ratatui::style::Modifier;

    fn make_entry(
        key_sequence: &str,
        description: &str,
        scope: &str,
        category: &str,
    ) -> KeymapEntry {
        let search_text = format!("{key_sequence} {description}");
        KeymapEntry {
            key_sequence: key_sequence.to_owned(),
            description: description.to_owned(),
            scope: scope.to_owned(),
            category: category.to_owned(),
            command: crate::protocol::Intent::Quit,
            search_text,
        }
    }

    #[rstest::rstest]
    fn display_label_returns_search_text() {
        let entry = make_entry("gg", "scroll to top", "Normal", "Navigation");
        assert_eq!(entry.display_label(), "gg scroll to top");
    }

    #[rstest::rstest]
    fn render_row_contains_key() {
        let entry = make_entry("gg", "scroll to top", "Normal", "Navigation");
        let line = entry.render_row(false);
        let text: String = line.spans.iter().map(|s| &*s.content).collect();
        assert!(text.contains("gg"), "should contain key sequence");
    }

    #[rstest::rstest]
    fn render_row_contains_description() {
        let entry = make_entry("gg", "scroll to top", "Normal", "Navigation");
        let line = entry.render_row(false);
        let text: String = line.spans.iter().map(|s| &*s.content).collect();
        assert!(text.contains("scroll to top"), "should contain description");
    }

    #[rstest::rstest]
    fn render_row_contains_scope() {
        let entry = make_entry("gg", "scroll to top", "Normal", "Navigation");
        let line = entry.render_row(false);
        let text: String = line.spans.iter().map(|s| &*s.content).collect();
        assert!(text.contains("[Normal]"), "should contain scope");
    }

    #[rstest::rstest]
    fn render_row_contains_category() {
        let entry = make_entry("gg", "scroll to top", "Normal", "Navigation");
        let line = entry.render_row(false);
        let text: String = line.spans.iter().map(|s| &*s.content).collect();
        assert!(text.contains("Navigation"), "should contain category");
    }

    #[rstest::rstest]
    fn render_row_selected_has_dark_gray_background() {
        let entry = make_entry("q", "quit", "Normal", "General");
        let line = entry.render_row(true);
        let key_span = &line.spans[0];
        assert_eq!(key_span.style.bg, Some(Color::DarkGray));
    }

    #[rstest::rstest]
    fn render_row_unselected_has_reset_background() {
        let entry = make_entry("q", "quit", "Normal", "General");
        let line = entry.render_row(false);
        let key_span = &line.spans[0];
        assert_eq!(key_span.style.bg, Some(Color::Reset));
    }

    #[rstest::rstest]
    fn render_row_key_is_yellow_bold() {
        let entry = make_entry("q", "quit", "Normal", "General");
        let line = entry.render_row(false);
        let key_span = &line.spans[0];
        assert_eq!(key_span.style.fg, Some(Color::Yellow));
        assert!(key_span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[rstest::rstest]
    fn render_row_pads_short_key_sequences() {
        let entry = make_entry("q", "quit", "Normal", "General");
        let line = entry.render_row(false);
        let key_span = &line.spans[0];
        assert!(
            key_span.content.len() >= 8,
            "key span should be padded to at least 8 chars"
        );
    }

    #[rstest::rstest]
    fn search_text_combines_key_sequence_and_description() {
        let entry = make_entry("<c-p>", "open picker keymap", "Normal", "General");
        assert!(entry.search_text.contains("<c-p>"));
        assert!(entry.search_text.contains("open picker keymap"));
    }

    #[rstest::rstest]
    fn render_row_with_empty_match_indices_same_as_render_row() {
        let entry = make_entry("gg", "scroll to top", "Normal", "Navigation");
        let normal = entry.render_row(false);
        let highlighted = entry.render_row_with_highlight(false, &[]);
        assert_eq!(normal.spans.len(), highlighted.spans.len());
        for (n, h) in normal.spans.iter().zip(highlighted.spans.iter()) {
            assert_eq!(n.content, h.content);
            assert_eq!(n.style, h.style);
        }
    }

    #[rstest::rstest]
    fn render_row_with_highlight_applies_gray_bg_to_matched_chars() {
        let entry = make_entry("q", "quit", "Normal", "General");
        #[expect(
            clippy::single_range_in_vec_init,
            reason = "genuinely want a slice containing one Range<usize>"
        )]
        let highlights: &[std::ops::Range<usize>] = &[0..1];
        let line = entry.render_row_with_highlight(false, highlights);
        let has_highlight = line
            .spans
            .iter()
            .any(|s| s.style.bg == Some(Color::DarkGray));
        assert!(
            has_highlight,
            "expected at least one span with gray background"
        );
    }

    #[rstest::rstest]
    fn render_row_with_highlight_preserves_unmatched_chars() {
        let entry = make_entry("gg", "scroll to top", "Normal", "Navigation");
        #[expect(
            clippy::single_range_in_vec_init,
            reason = "genuinely want a slice containing one Range<usize>"
        )]
        let highlights: &[std::ops::Range<usize>] = &[0..1];
        let line = entry.render_row_with_highlight(false, highlights);
        let text: String = line.spans.iter().map(|s| &*s.content).collect();
        assert!(text.contains("gg"), "should still contain 'gg'");
        assert!(
            text.contains("scroll to top"),
            "should still contain description"
        );
    }
}
