//! Tree item trait — the consumer-facing contract for tree-structured picker items.
//!
//! Consumers implement [`TreeItem`] for their domain type (e.g., `SessionTreeEntry`,
//! file entries). The widget uses [`TreeItem::display_label`] for fuzzy matching and
//! [`TreeItem::render_row`] for styled display in the picker list. Tree structure
//! is expressed via [`TreeItem::id`] and [`TreeItem::parent_id`].

use std::ops::Range;

use ratatui::text::Line;

/// An item that can be displayed and selected in a tree-structured picker.
///
/// Items form a tree via `id` / `parent_id` relationships. Items with
/// `parent_id` returning `None` are roots. The tree picker uses DFS traversal
/// to render items in tree order.
///
/// # ID Contract
///
/// - [`id`](TreeItem::id) must return a unique string for each item.
/// - [`parent_id`](TreeItem::parent_id) must return the ID of the parent item,
///   or `None` for root items.
/// - If `parent_id` references an ID not present in the item list, the item
///   is treated as a root (orphan guard).
///
/// # Examples
///
/// ```ignore
/// struct FileEntry {
///     path: String,
///     parent_path: Option<String>,
///     name: String,
/// }
///
/// impl TreeItem for FileEntry {
///     fn id(&self) -> &str { &self.path }
///     fn parent_id(&self) -> Option<&str> { self.parent_path.as_deref() }
///     fn display_label(&self) -> &str { &self.name }
///     fn render_row(&self, is_selected: bool) -> Line<'static> {
///         Line::from(self.name.clone())
///     }
/// }
/// ```
pub trait TreeItem: std::fmt::Debug + 'static {
    /// Returns the unique identifier for this item.
    ///
    /// Used to resolve parent/child relationships.
    fn id(&self) -> &str;

    /// Returns the ID of this item's parent, or `None` for root items.
    fn parent_id(&self) -> Option<&str>;

    /// Returns searchable text used for fuzzy matching.
    ///
    /// Should contain all text the user might search by.
    fn display_label(&self) -> &str;

    /// Renders this item as a styled line for display in the picker.
    ///
    /// `is_selected` indicates whether this row is currently highlighted.
    /// The tree prefix is prepended by the widget ��� renderers should NOT
    /// include tree connectors.
    fn render_row(&self, is_selected: bool) -> Line<'static>;

    /// Renders this item with fuzzy match highlighting.
    ///
    /// `is_selected` indicates whether this row is currently highlighted.
    /// `match_indices` contains sorted, non-overlapping byte ranges within
    /// [`display_label`](Self::display_label) that matched the filter.
    /// When `match_indices` is empty (no active filter), delegates to [`render_row`](Self::render_row).
    fn render_row_with_highlight(
        &self,
        is_selected: bool,
        match_indices: &[Range<usize>],
    ) -> Line<'static>;
}
