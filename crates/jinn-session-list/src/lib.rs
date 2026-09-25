//! Owner-neutral session-list projection and visible-tree algorithms.

mod model;
mod tree;
mod tree_node;
mod visual_parents;

pub use model::{SessionEntry, SessionEntryKind};
pub use tree::visible_session_tree;
pub use tree_node::{SessionTreeNode, visible_session_at};
pub use visual_parents::{clear_visual_parents_on_load, repair_visual_parents_on_removal};
