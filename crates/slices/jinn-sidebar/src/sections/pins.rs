//! Pinned entries sidebar section.

pub mod pins_section;
pub mod validator;

pub use jinn_sidebar_msg::PinsState;
pub use pins_section::PinsSection;
pub use pins_section::{navigate, pins_section_content_height, receive_cursor};

#[cfg(test)]
mod pins_section_tests;
