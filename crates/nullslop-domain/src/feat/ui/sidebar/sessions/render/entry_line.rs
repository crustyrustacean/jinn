//! Pure helper functions for building a single session entry line.
//!
//! Each function is pure (no side effects, no `&mut self`) and takes explicit
//! parameters so it can be unit-tested in isolation.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use throbber_widgets_tui::ThrobberState;

use crate::feat::session::chat_entry::ChatEntryKind;
use crate::feat::session::chat_session::{ChatSessionState, SessionPhase};
use crate::feat::theme::Theme;
use crate::feat::ui::sidebar::sessions::state::SessionTreeEntry;

use super::super::{ACTIVE_PREFIX, INACTIVE_PREFIX};
use super::truncate::truncate_str;

/// Render configuration — layout, theme, animation.
/// Groups everything the entry renderer needs that isn't session data.
pub(crate) struct EntryRenderConfig<'a> {
    pub(crate) max_title_len: usize,
    pub(crate) theme: &'a Theme,
    pub(crate) throbber_state: &'a ThrobberState,
}

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
pub(crate) fn tree_prefix(entry: &SessionTreeEntry) -> String {
    if entry.depth == 0 {
        return String::new();
    }
    // Skip ancestor_continuations[0] — the root-level continuation.
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
/// Reads all session properties from the live `ChatSessionState`.
/// Tree layout comes from `SessionTreeEntry`.
/// Render config groups theme, animation, and layout parameters.
pub(crate) fn assemble_entry_line(
    tree: &SessionTreeEntry,
    session: &ChatSessionState,
    is_active: bool,
    is_selected: bool,
    config: &EntryRenderConfig<'_>,
) -> Line<'static> {
    let is_idle = matches!(session.phase(), SessionPhase::Idle);
    let last_entry_is_error = session
        .history()
        .last()
        .is_some_and(|e| matches!(&e.kind, ChatEntryKind::Error(..)));
    let title = session.title().unwrap_or("Untitled Session");
    let is_judge = session.is_judge();
    let judge_attached = session.judge().as_ref().map(|m| m.is_attached);

    let indicator = indicator_span(is_idle, config.throbber_state);
    let arrow = arrow_span(is_active, config.theme);
    let tree_str = tree_prefix(tree);
    let tree_len = tree_str.len();
    let mut style = entry_title_style(is_selected, is_active, last_entry_is_error, config.theme);

    let display_title = if is_judge {
        let truncated = truncate_str(title, config.max_title_len.saturating_sub(tree_len + 2));
        format!("⚖ {truncated}")
    } else {
        truncate_str(title, config.max_title_len.saturating_sub(tree_len))
    };

    // Overlay judge-specific colors.
    if let Some(attached) = judge_attached {
        if attached {
            style = style.bg(Color::Rgb(80, 50, 120)).fg(Color::White);
        } else {
            style = style.fg(Color::Rgb(160, 130, 200));
        }
    }

    let mut spans = vec![indicator, Span::raw(" "), arrow];
    if !tree_str.is_empty() {
        spans.push(Span::styled(
            tree_str,
            Style::default().fg(config.theme.muted_text),
        ));
    }
    spans.push(Span::styled(display_title, style));
    Line::from(spans)
}
