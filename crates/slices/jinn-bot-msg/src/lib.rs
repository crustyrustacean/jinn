//! Bot run parameters and the report log.
//!
//! A bot is a session that references a parent without inheriting its
//! conversation. These types describe *how* a bot runs — when it
//! fires, what it does to its context beforehand, whether it is still being
//! composed, and what it has concluded so far. The four facts are named for
//! the four rows of the properties popup, and nothing here combines them:
//! a bot that is still being composed can hold a behavior and a
//! trigger, they simply do not apply until it stops being composed.

mod behavior;
mod properties;
mod report;
mod report_picker;
mod saved_picker;

pub use crate::behavior::{BotBehavior, BotModelSetting, BotTrigger};
pub use crate::properties::{
    BEHAVIOR_CHOICES, BotPropertiesState, MODEL_CHOICES, OriginalValues, PickDirection,
    PopupStatus, PropertyField, SET_MODE_CHOICES, SetField, SetMode, TRIGGER_CHOICES,
    bot_properties_scope, bot_properties_slot, bot_seed_template_scope, pick_behavior,
    pick_model_setting, pick_trigger,
};
pub use crate::report::BotReport;

/// The built-in tools that exist only inside a bot.
///
/// A bot reports its conclusions and wakes its parent; neither
/// action means anything for an ordinary session, so the definitions are
/// filtered out of every non-bot session's tool list rather than
/// being offered and refused at call time.
pub const BOT_TOOL_NAMES: &[&str] = &["conclude", "notify_parent"];

/// Whether a tool name belongs to the bot-only family.
#[must_use]
pub fn is_bot_tool(name: &str) -> bool {
    BOT_TOOL_NAMES.contains(&name)
}

/// Whether `def` belongs to the bot-only family.
///
/// Takes the whole definition rather than a bare name so every reader —
/// the prompt assembler, the tool picker — asks the same question of the
/// same value, instead of one filtering by name and the other forgetting.
#[must_use]
pub fn is_bot_tool_definition(def: &jinn_core_types::ToolDefinition) -> bool {
    is_bot_tool(&def.name)
}

pub use crate::report_picker::{
    BotReportPickerState, bot_report_picker_scope, bot_report_picker_slot,
};
pub use crate::saved_picker::{
    BotSavedPickerState, SavedBotSummary, bot_saved_picker_scope, bot_saved_picker_slot,
};

#[cfg(test)]
mod tests;

/// The placeholder a bot's seed template uses to refer to its last report.
pub const PRIOR_REPORT_PLACEHOLDER: &str = "<prior_report>";

/// The placeholder a bot's seed template uses to refer to its parent
/// session's id.
pub const PARENT_ID_PLACEHOLDER: &str = "<parent_id>";

/// What the placeholder becomes on a run that has no prior report.
///
/// A first run has nothing to fold in, so the token is replaced with a plain
/// sentence rather than left in place. Shipping the raw token would put a
/// template instruction in front of the model as though the user had written
/// it, and would read as a question about a report that does not exist.
pub const NO_PRIOR_REPORT_TEXT: &str = "this is the first run, so there is no prior report";

/// Stands in for the parent session id when a bot has no parent on
/// record.
///
/// Reads as a sentence in the prompt and tells the agent the id is not
/// available, rather than leaving the line trailing off after "is" — a
/// prompt with a dangling label is a prompt the model tries to interpret.
pub const NO_PARENT_SESSION_TEXT: &str = "unavailable";

/// The seed text a fresh attached bot starts with when the user has not
/// written one: one line naming both tokens.
///
/// Describes the situation rather than prescribing a kind of work, so the
/// default reads sensibly for a bot that is not inspecting code. This is
/// also the fallback a hand-written `[[bot.entry]]` without a
/// `seed_template` key gets — entries are attached bots unless edited
/// otherwise, so the parent line is the useful default.
#[must_use]
pub fn default_seed_template() -> String {
    format!(
        "The parent session's id is {PARENT_ID_PLACEHOLDER}. The previous run of this bot reported: {PRIOR_REPORT_PLACEHOLDER}."
    )
}

/// The seed text a standalone bot starts with: nothing.
///
/// A standalone bot has no parent line to name and no prior report to
/// refer to on a first run, so its template is empty and the dispatched
/// message *is* the prompt. [`default_seed_template`] remains the
/// attached-bot default and the serde fallback.
#[must_use]
pub fn empty_seed_template() -> String {
    String::new()
}
