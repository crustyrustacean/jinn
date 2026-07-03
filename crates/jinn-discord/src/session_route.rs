//! Session routing helpers — shared between the gateway message handler and
//! tests.
//!
//! The pure routing decision (phase → enqueue vs steer) lives in
//! [`jinn_domain::feat::discord::route`]; this module is reserved for any
//! future composite helpers that need both the route decision and the bus
//! publish step together. Currently empty of logic.
