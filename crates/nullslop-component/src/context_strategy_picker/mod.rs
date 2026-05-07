//! Context assembly strategy picker — entry types, loading, and sorting.
//!
//! This component provides the data types for the strategy picker UI.
//! The picker reuses the shared `Picker*` commands and `SelectionState<StrategyEntry>`.
//! Handler wiring and registration happen in Phase 3.

pub mod entries;
