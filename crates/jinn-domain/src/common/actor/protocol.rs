//! Actor protocol - commands, events, and dynamic messages.

pub mod command;
pub mod dynamic_command;
#[cfg(test)]
mod dynamic_command_tests;
pub mod dynamic_event;
#[cfg(test)]
mod dynamic_event_tests;
pub mod event;
