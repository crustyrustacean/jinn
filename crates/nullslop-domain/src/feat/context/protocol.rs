//! Context protocol — commands, events, strategy identification, and prompt templates.

pub mod command;
pub mod event;
pub mod prompt_template;
pub mod strategy_id;

pub use prompt_template::PromptTemplate;
