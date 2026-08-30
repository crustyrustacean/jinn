//! Interactive-term protocol — ask messages and events for the coordinator.
//!
//! The three tools `ask` this actor directly (request/reply, mirroring the
//! `restart_mcp_server` tool); the takeover UI sends [`SetTermControl`] and
//! receives [`TermScreenUpdated`] events.

pub mod command;
pub mod event;
