//! Provider domain: commands, events, and LLM message types.

pub use crate::feat::provider::llm_message::LlmMessage;
pub use crate::feat::provider::protocol::command::{
    CancelStream, ProviderSwitch, RefreshModels, RescanPromptTemplates, SendMessage,
    SendToLlmProvider,
};
pub use crate::feat::provider::protocol::event::{
    ModelsRefreshed, PromptTemplatesLoaded, ProviderSwitched, StreamCompleted,
    StreamCompletedReason, StreamToken,
};
