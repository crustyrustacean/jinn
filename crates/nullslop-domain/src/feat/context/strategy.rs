//! Strategy implementations and shared types.

pub mod compaction;
pub mod compaction_data;
pub mod compaction_prompt;
#[cfg(test)]
mod compaction_tests;
pub mod discovery;
pub mod factory;
pub mod passthrough;
pub mod sliding_window;
#[cfg(test)]
mod sliding_window_tests;
pub mod token_budget;
#[cfg(test)]
mod token_budget_tests;
pub mod token_estimator;
#[cfg(test)]
mod token_estimator_tests;
pub mod turn_grouping;
#[cfg(test)]
mod turn_grouping_tests;
pub mod types;
