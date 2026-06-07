//! Reusable search+filter+select widget for ratatui TUI applications.
//!
//! This crate provides a [`PickerItem`] trait and a [`SelectionState`] generic state machine.
//! Consumers bring their own item types, define how items display and render, and the widget
//! handles fuzzy filtering, cursor navigation, and selection management.
//!
//! Commands, handlers, and keymap wiring live in consumer crates - this crate is purely
//! the state and types layer.

pub mod highlight;
pub mod item;
pub mod picker_ops;
#[cfg(test)]
mod picker_ops_tests;
pub mod preview_content;
pub mod preview_widget;
pub mod state;
#[cfg(test)]
mod state_tests;
pub mod tree_item;
pub mod tree_state;
#[cfg(test)]
mod tree_state_tests;
pub mod tree_widget;
#[cfg(test)]
mod tree_widget_tests;
pub mod widget;
#[cfg(test)]
mod widget_tests;

pub use highlight::{
    PICKER_HIGHLIGHT_STYLE, highlight_style, highlight_text, highlight_text_with_bg,
};
pub use item::{MatchRanges, PickerItem};
pub use picker_ops::PickerOps;
pub use preview_content::{PreviewCache, PreviewContent};
pub use preview_widget::PreviewSelectionWidget;
pub use state::SelectionState;
pub use tree_item::TreeItem;
pub use tree_state::{TreePickerState, VisibleEntry};
pub use tree_widget::TreePickerWidget;
pub use widget::{
    PICKER_H_PAD_FRAC, PICKER_MAX_HEIGHT_FRAC, PICKER_MIN_WIDTH, SelectionColors, SelectionWidget,
    compute_popup_rect,
};
