//! Attendant run parameters and the report log.
//!
//! An attendant is a session that references a parent without inheriting its
//! conversation. These types describe *how* an attendant runs — when it fires,
//! what it does to its context beforehand, and what it has concluded so far.

mod activation;
mod properties;
mod report;
mod report_picker;

pub use crate::activation::{AttendantActivation, AttendantContextPolicy, AttendantTrigger};
pub use crate::properties::{
    ACTIVATION_CHOICES, AttendantPropertiesState, OriginalValues, PickDirection, PropertyField,
    TRIGGER_CHOICES, attendant_properties_scope, attendant_properties_slot,
    attendant_seed_template_scope, pick_activation, pick_trigger,
};
pub use crate::report::AttendantReport;

/// The built-in tools that exist only inside an attendant.
///
/// An attendant reports its conclusions and wakes its parent; neither
/// action means anything for an ordinary session, so the definitions are
/// filtered out of every non-attendant session's tool list rather than
/// being offered and refused at call time.
pub const ATTENDANT_TOOL_NAMES: &[&str] = &["conclude", "notify_parent"];

/// Whether a tool name belongs to the attendant-only family.
#[must_use]
pub fn is_attendant_tool(name: &str) -> bool {
    ATTENDANT_TOOL_NAMES.contains(&name)
}

pub use crate::report_picker::{
    AttendantReportPickerState, attendant_report_picker_scope, attendant_report_picker_slot,
};

#[cfg(test)]
mod tests;

/// The placeholder an attendant's seed template uses to refer to its last report.
pub const PRIOR_REPORT_PLACEHOLDER: &str = "<prior report>";

/// What the placeholder becomes on a run that has no prior report.
///
/// A first run has nothing to fold in, so the token is replaced with a plain
/// sentence rather than left in place. Shipping the raw token would put a
/// template instruction in front of the model as though the user had written
/// it, and would read as a question about a report that does not exist.
pub const NO_PRIOR_REPORT_TEXT: &str = "this is the first run, so there is no prior report";

/// Labels the parent session id that every seed prompt carries.
///
/// Appended to the user's own template rather than substituted into it, so
/// nothing the user typed is reordered or rewritten. An attendant runs in
/// its own session and has no other way to find the transcript it reports
/// on — a session-search tool needs an id to search by.
pub const PARENT_SESSION_HEADER: &str = "The parent session's id is";

/// Stands in for the parent session id when an attendant has no parent on
/// record.
///
/// Reads as a sentence in the prompt and tells the agent the id is not
/// available, rather than leaving the line trailing off after "is" — a
/// prompt with a dangling label is a prompt the model tries to interpret.
pub const NO_PARENT_SESSION_TEXT: &str = "unavailable";

/// The seed text a fresh attendant starts with when the user has not written one.
///
/// Describes the situation rather than prescribing a kind of work, so the
/// default reads sensibly for an attendant that is not inspecting code.
#[must_use]
pub fn default_seed_template() -> String {
    format!("The previous run of this attendant reported: {PRIOR_REPORT_PLACEHOLDER}.")
}
