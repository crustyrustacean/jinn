//! Workflow tab UI state — persisted across frames in `FrontendState`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU16, Ordering};

use jinn_workflow::spatial_layout::SpatialRect;

use crate::feat::chat_input::ChatInputBoxState;

/// Workflow tab UI state — persisted across frames in `FrontendState`.
///
/// OWNER: IntentHandler (selection, inspector toggle, cancel prompt).
#[derive(Debug, Default)]
pub struct WorkflowUiState {
    /// Currently selected node name, if any.
    pub selected_node: Option<String>,
    /// Viewport horizontal offset (cells).
    pub viewport_offset_x: i32,
    /// Viewport vertical offset (cells).
    pub viewport_offset_y: i32,
    /// Whether the sticky inspector popup is showing.
    pub inspector_open: bool,
    /// Scroll position within the inspector popup (lines from top).
    pub inspector_scroll: u16,
    /// The actual clamped scroll position after rendering.
    ///
    /// Written by the renderer each frame, read by intent handlers
    /// so repeated "scroll down" inputs don't accumulate past the limit.
    pub inspector_scroll_rendered: AtomicU16,
    /// Whether the "Press ESC again to cancel" prompt is showing.
    pub cancel_prompt: bool,
    /// Cached spatial index: node name → bounding rect in content coordinates.
    ///
    /// Recomputed lazily when empty and a spatial navigation intent fires.
    /// Cleared when the active workflow changes.
    pub node_rects: HashMap<String, SpatialRect>,
    /// The text editing buffer for the workflow node being edited.
    /// Reuses `ChatInputBoxState` for cursor, wrapping, and scroll management.
    pub input_buffer: ChatInputBoxState,
    /// The name of the source node currently being edited, if any.
    pub editing_node: Option<String>,
}

impl Clone for WorkflowUiState {
    fn clone(&self) -> Self {
        Self {
            selected_node: self.selected_node.clone(),
            viewport_offset_x: self.viewport_offset_x,
            viewport_offset_y: self.viewport_offset_y,
            inspector_open: self.inspector_open,
            inspector_scroll: self.inspector_scroll,
            inspector_scroll_rendered: AtomicU16::new(
                self.inspector_scroll_rendered.load(Ordering::Relaxed),
            ),
            cancel_prompt: self.cancel_prompt,
            node_rects: self.node_rects.clone(),
            input_buffer: self.input_buffer.clone(),
            editing_node: self.editing_node.clone(),
        }
    }
}
