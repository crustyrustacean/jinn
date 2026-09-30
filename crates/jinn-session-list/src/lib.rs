//! Owner-neutral session-list projection and visible-tree algorithms.

mod closure;
mod model;
mod tree;
mod tree_node;
mod visual_parents;

pub use closure::descendant_closure;
pub use model::{SessionEntry, SessionEntryKind};
pub use tree::visible_session_tree;
pub use tree_node::{SessionTreeNode, visible_index_of, visible_session_at};
pub use visual_parents::{clear_visual_parents_on_load, repair_visual_parents_on_removal};
