//! Tab domain: tab management types, active tab state, and tab navigation commands.

mod active_tab;
mod command;
mod tab_direction;

pub use active_tab::ActiveTab;
pub use tab_direction::TabDirection;
