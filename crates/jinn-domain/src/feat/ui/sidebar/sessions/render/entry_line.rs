//! Pure helper functions for building a single session entry line.
//!
//! Each function is pure (no side effects, no `&mut self`) and takes explicit
//! parameters so it can be unit-tested in isolation.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use throbber_widgets_tui::ThrobberState;
use unicode_segmentation::UnicodeSegmentation;

use crate::feat::theme::Theme;
use crate::feat::ui::sidebar::sessions::state::{SessionEntry, SessionEntryKind};

use super::super::{ACTIVE_PREFIX, INACTIVE_PREFIX};
use super::truncate::truncate_str;

/// Builds the animated throbber indicator span for a session entry.
///
/// Returns a blank space when the session is idle, or an animated braille
/// character when the session is working.
pub(crate) fn indicator_span(is_idle: bool, throbber_state: &ThrobberState) -> Span<'static> {
    if is_idle {
        Span::raw(" ")
    } else {
        let set = throbber_widgets_tui::symbols::throbber::BRAILLE_EIGHT;
        let mut idx = throbber_state.index();
        let len = set.symbols.len() as i8;
        idx %= len;
        if idx < 0 {
            idx += len;
        }
        let ch = set.symbols[idx as usize];
        Span::styled(ch.to_string(), Style::default().fg(Color::Cyan))
    }
}

/// Builds the arrow prefix span indicating whether a session is active.
///
/// Active sessions display `▸ `, inactive sessions display `  ` (aligned).
pub(crate) fn arrow_span(is_active: bool, theme: &Theme) -> Span<'static> {
    if is_active {
        Span::styled(
            ACTIVE_PREFIX.to_owned(),
            Style::default().fg(theme.primary_text),
        )
    } else {
        Span::styled(INACTIVE_PREFIX.to_owned(), Style::default())
    }
}

/// Computes the title style based on entry state.
///
/// Priority: error+selected → red+reversed, error → red, selected → reversed,
/// active → primary text, default → muted text.
pub(crate) fn entry_title_style(
    is_selected: bool,
    is_active: bool,
    last_entry_is_error: bool,
    theme: &Theme,
) -> Style {
    if last_entry_is_error {
        if is_selected {
            Style::default()
                .fg(Color::Red)
                .add_modifier(Modifier::REVERSED)
        } else {
            Style::default().fg(Color::Red)
        }
    } else if is_selected {
        Style::default().add_modifier(Modifier::REVERSED)
    } else if is_active {
        Style::default().fg(theme.primary_text)
    } else {
        Style::default().fg(theme.muted_text)
    }
}

/// Builds the tree connector prefix for a session entry.
///
/// For root entries (depth 0), returns an empty string.
/// For non-root entries, constructs non-compacted 3-char-wide segments:
/// - Skips `ancestor_continuations[0]` (root-level) since roots have no prefix.
/// - For each intermediate ancestor level: `│  ` if continuing, `   ` if not
/// - For the entry's own level: `├─ ` if has younger siblings, `└─ ` if last child
pub(crate) fn tree_prefix(entry: &SessionEntry) -> String {
    if entry.depth == 0 {
        return String::new();
    }
    // Skip ancestor_continuations[0] - the root-level continuation.
    // Roots have no tree prefix, so there's nothing for that │ to connect to.
    let mut prefix = String::with_capacity(entry.depth * 3);
    for &continues in &entry.ancestor_continuations[1..] {
        prefix.push_str(if continues { "│  " } else { "   " });
    }
    prefix.push_str(if entry.is_last_child {
        "└─ "
    } else {
        "├─ "
    });
    prefix
}

/// Assembles a complete session entry line from its components.
///
/// Combines the throbber indicator, arrow prefix, tree connector prefix,
/// and styled truncated title into a single [`Line`] ready for rendering.
pub(crate) fn assemble_entry_line(
    entry: &SessionEntry,
    is_selected: bool,
    max_title_len: usize,
    throbber_state: &ThrobberState,
    theme: &Theme,
) -> Line<'static> {
    match entry.kind {
        SessionEntryKind::Session => {
            assemble_session_line(entry, is_selected, max_title_len, throbber_state, theme)
        }
        SessionEntryKind::Plugin { enabled } => {
            assemble_workflow_line(enabled, entry, is_selected, max_title_len, theme)
        }
    }
}

/// Renders a session entry line (indicator + arrow + tree + styled title).
fn assemble_session_line(
    entry: &SessionEntry,
    is_selected: bool,
    max_title_len: usize,
    throbber_state: &ThrobberState,
    theme: &Theme,
) -> Line<'static> {
    let indicator = indicator_span(entry.is_idle, throbber_state);
    let arrow = arrow_span(entry.is_active, theme);
    let tree = tree_prefix(entry);
    let tree_len = tree.graphemes(true).count();
    let style = entry_title_style(
        is_selected,
        entry.is_active,
        entry.last_entry_is_error,
        theme,
    );
    let display_title = truncate_str(&entry.title, max_title_len.saturating_sub(tree_len));
    let mut spans = vec![indicator, Span::raw(" "), arrow];
    if !tree.is_empty() {
        spans.push(Span::styled(tree, Style::default().fg(theme.muted_text)));
    }
    spans.push(Span::styled(display_title, style));
    Line::from(spans)
}

/// Renders a workflow entry line (lightning bolt icon + space + tree + dimmed title).
///
/// Workflow entries are informational children of a session. They show the
/// plugin label with a lightning bolt icon prefix. Disabled workflows get an additional
/// DIM modifier on the title.
fn assemble_workflow_line(
    enabled: bool,
    entry: &SessionEntry,
    is_selected: bool,
    max_title_len: usize,
    theme: &Theme,
) -> Line<'static> {
    // Lightning bolt icon replaces the throbber indicator.
    let icon = Span::styled("\u{26A1}", Style::default().fg(theme.muted_text));
    // No arrow for workflows — they are never active.
    let no_arrow = Span::raw(" ");
    let tree = tree_prefix(entry);
    let tree_len = tree.graphemes(true).count();

    let title_style = if enabled {
        if is_selected {
            Style::default()
                .add_modifier(Modifier::REVERSED)
                .fg(theme.muted_text)
        } else {
            Style::default().fg(theme.muted_text)
        }
    } else if is_selected {
        Style::default()
            .fg(theme.muted_text)
            .add_modifier(Modifier::REVERSED | Modifier::DIM)
    } else {
        Style::default()
            .fg(theme.muted_text)
            .add_modifier(Modifier::DIM)
    };

    let prefix = if enabled { "" } else { "\u{2717} " };
    let full_title = format!("{prefix}{}", entry.title);
    let display_title = truncate_str(&full_title, max_title_len.saturating_sub(tree_len));

    let mut spans = vec![icon, Span::raw(" "), no_arrow];
    if !tree.is_empty() {
        spans.push(Span::styled(tree, Style::default().fg(theme.muted_text)));
    }
    spans.push(Span::styled(display_title, title_style));
    Line::from(spans)
}
