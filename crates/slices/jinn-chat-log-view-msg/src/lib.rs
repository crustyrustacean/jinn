pub mod chat_log_view_state;
pub mod layout;
pub mod visual_item;

pub use chat_log_view_state::*;
pub use layout::{
    ArmLayoutDeadline, ArmPreviewDeadline, ChatLogLayoutComputed, Escalated, LayoutChatSession,
    LayoutDeadlineExpired, MeasuredEntryCount, PREVIEW_ENTRY_COUNT, PREVIEW_MAX_LINES,
    PreviewSessionRequested, SessionPreviewRendered,
};
pub use visual_item::{
    DEFAULT_MIN_COLLAPSE_COUNT, PROXIMITY_COUNT, VisualItem, build_visual_items,
    entry_id_from_visual_item, resolve_entry_id_to_vi_index,
};
