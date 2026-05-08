//! Provider domain: commands, events, and LLM message types.

mod command;
mod convert;
mod event;
mod message;

pub use command::{
    CancelStream, ProviderSwitch, RefreshModels, RescanPromptTemplates, SendMessage,
    SendToLlmProvider,
};
pub use convert::entries_to_messages;
pub use event::{
    ModelsRefreshed, PromptTemplatesLoaded, ProviderSwitched, StreamCompleted,
    StreamCompletedReason, StreamToken,
};
pub use message::LlmMessage;
