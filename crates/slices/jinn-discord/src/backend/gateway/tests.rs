//! Tests for the jinn-discord gateway.
//!
//! The poise gateway + Discord websocket aren't unit-testable here (they need
//! a live Discord connection). The pure-logic pieces they depend on
//! (`split_message`, `read_final_reply`, `route_decision`)
//! are tested in `jinn-domain`'s `feat/discord` tests. This module holds the
//! few harnessable pieces that live in this crate.
