//! Picker entry for the project picker - one row per curated directory.

use jinn_selection_widget::PickerItem;
use ratatui::text::{Line, Span};

use jinn_common::shorten_path;
use jinn_picker::picker_style::{active_marker, selected_style};
use jinn_preferences_config::schemas::ProjectConfig;
use jinn_theme::Theme;
use marked_path::{Absolute, MarkedPathBuf};

/// A curated project directory shown in the project picker.
///
/// `display` is the tilde-compressed form (precomputed once at load so filtering
/// and sorting operate on what the user sees); `path` is the canonical dir used
/// to set the new session's CWD on confirm.
#[derive(Debug, Clone)]
pub struct ProjectEntry {
    /// The canonical directory path (what the new session's CWD is set to).
    pub path: MarkedPathBuf<Absolute>,
    /// The tilde-compressed display string (precomputed from `path`).
    pub display: String,
    /// Path as a string, cached for the `TreeItem` identity contract.
    pub id_string: String,
    /// Theme for rendering.
    pub theme: Theme,
}

impl ProjectEntry {
    /// Build an entry from a directory path, precomputing the display string.
    #[must_use]
    pub fn new(path: MarkedPathBuf<Absolute>, theme: Theme) -> Self {
        let display = shorten_path(&path);
        Self {
            id_string: path.to_string(),
            path,
            display,
            theme,
        }
    }
}

impl PickerItem for ProjectEntry {
    fn display_label(&self) -> &str {
        &self.display
    }

    fn render_row(&self, is_selected: bool) -> Line<'static> {
        render_project_row(&self.display, is_selected, &[], &self.theme)
    }

    fn render_row_with_highlight(
        &self,
        is_selected: bool,
        match_indices: &[std::ops::Range<usize>],
    ) -> Line<'static> {
        render_project_row(&self.display, is_selected, match_indices, &self.theme)
    }
}

/// Renders a project picker row with selection styling.
pub fn render_project_row(
    display: &str,
    is_selected: bool,
    match_indices: &[std::ops::Range<usize>],
    theme: &Theme,
) -> Line<'static> {
    let base_style = selected_style(is_selected, theme);
    let mut spans = vec![active_marker(is_selected, theme)];

    if match_indices.is_empty() {
        spans.push(Span::styled(display.to_owned(), base_style));
    } else {
        spans.extend(jinn_selection_widget::highlight_text_with_bg(
            display,
            base_style,
            match_indices,
            theme.picker_highlight_bg,
        ));
    }

    Line::from(spans)
}

/// Build entries for the project picker from the curated project list.
///
/// Entries are sorted by their display string so the list is stable and
/// alphabetical regardless of `jinn.toml` ordering.
pub fn project_entries(projects: &[ProjectConfig], theme: &Theme) -> Vec<ProjectEntry> {
    let mut entries: Vec<_> = projects
        .iter()
        .filter_map(|p| {
            // `~`-prefixed entries expand against the process home; the
            // schema field is the user-editable TOML form (std::path).
            let expanded = if let Ok(rest) = p.path.strip_prefix("~") {
                let home = dirs::home_dir()?;
                Some(home.join(rest))
            } else {
                Some(p.path.clone())
            };
            let raw = expanded?;
            MarkedPathBuf::<Absolute>::new(raw).ok()
        })
        .map(|path| ProjectEntry::new(path, theme.clone()))
        .collect();
    entries.sort_by(|a, b| a.display.cmp(&b.display));
    entries
}

impl jinn_selection_widget::TreeItem for ProjectEntry {
    fn id(&self) -> &str {
        // Paths are unique in `projects`; lossy is fine — this is identity only.
        &self.id_string
    }

    fn parent_id(&self) -> Option<&str> {
        None
    }

    fn display_label(&self) -> &str {
        &self.display
    }

    fn render_row(&self, is_selected: bool) -> Line<'static> {
        PickerItem::render_row(self, is_selected)
    }

    fn render_row_with_highlight(
        &self,
        is_selected: bool,
        match_indices: &[std::ops::Range<usize>],
    ) -> Line<'static> {
        PickerItem::render_row_with_highlight(self, is_selected, match_indices)
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use jinn_theme::default_theme;

    #[rstest::rstest]
    fn display_label_is_tilde_compressed_path() {
        // Given a project entry whose path sits under the home directory.
        let home = dirs::home_dir().expect("home dir available in test");
        let entry = ProjectEntry::new(
            MarkedPathBuf::<Absolute>::new(home.join("code").join("jinn"))
                .expect("home-joined path is absolute"),
            default_theme(),
        );

        // When reading its display label.
        // Then the display label is tilde-compressed with native separators
        // (PathBuf-built: `~/code/jinn` on Unix, `~\code\jinn` on Windows).
        let expected = std::path::Path::new("~")
            .join("code")
            .join("jinn")
            .display()
            .to_string();
        assert_eq!(entry.display_label(), expected);
    }

    #[rstest::rstest]
    fn render_row_unselected_has_leading_spaces() {
        // Given a project entry.
        let dir = jinn_testutil::tempdir();
        let path = dir.path.join("proj").expect("temp root joins a bare name");
        let entry = ProjectEntry::new(path.clone(), default_theme());

        // When rendering unselected.
        let text = entry.render_row(false).to_string();

        // Then it begins with the two-space inactive marker. The label is
        // the entry's `display_label` — `shorten_path`'s form — so the
        // expectation holds whatever the platform's home directory is.
        let expected = format!("  {}", entry.display_label());
        assert!(text.starts_with(expected.as_str()));
    }

    #[rstest::rstest]
    fn render_row_selected_has_arrow() {
        // Given a project entry.
        let dir = jinn_testutil::tempdir();
        let path = dir.path.join("proj").expect("temp root joins a bare name");
        let entry = ProjectEntry::new(path.clone(), default_theme());

        // When rendering selected.
        let text = entry.render_row(true).to_string();

        // Then it begins with the active arrow marker. The label is the
        // entry's `display_label` — `shorten_path`'s form — so the
        // expectation holds whatever the platform's home directory is.
        let expected = format!("> {}", entry.display_label());
        assert!(text.starts_with(expected.as_str()));
    }

    #[rstest::rstest]
    fn project_entries_sorted_by_display() {
        // Given projects in a non-alphabetical order. Their paths are
        // rooted at the platform's own filesystem root, so the display
        // strings keep the order the paths name without spelling a POSIX
        // literal.
        let root = std::path::Path::new(std::path::MAIN_SEPARATOR_STR);
        let projects = vec![
            ProjectConfig {
                path: root.join("zzz").clone(),
            },
            ProjectConfig {
                path: root.join("aaa").clone(),
            },
        ];

        // When building entries.
        let entries = project_entries(&projects, &default_theme());

        // Then they are sorted by display string.
        let aaa = root.join("aaa").to_string_lossy().to_string();
        let zzz = root.join("zzz").to_string_lossy().to_string();
        assert_eq!(entries[0].display, aaa);
        assert_eq!(entries[1].display, zzz);
    }

    #[rstest::rstest]
    fn project_entries_empty_input_returns_empty() {
        // Given no projects.
        // When building entries.
        let entries = project_entries(&[], &default_theme());

        // Then the result is empty.
        assert!(entries.is_empty());
    }
}
