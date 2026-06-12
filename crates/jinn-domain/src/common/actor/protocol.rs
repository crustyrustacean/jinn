//! Actor protocol - commands, events, and dynamic messages.

pub mod command;
pub mod dynamic_command;
//FIXME: disabled during actor migration
// #[cfg(test)]
#[cfg(any())]
mod dynamic_command_tests;
pub mod dynamic_event;
//FIXME: disabled during actor migration
// #[cfg(test)]
#[cfg(any())]
mod dynamic_event_tests;
pub mod event;
