//! Attendant run parameters and the report log.
//!
//! An attendant is a session that references a parent without inheriting its
//! conversation. These types describe *how* an attendant runs — when it fires,
//! what it does to its context beforehand, and what it has concluded so far.

mod activation;
mod properties;
mod report;
mod report_picker;

pub use crate::activation::{AttendantActivation, AttendantTrigger};
pub use crate::properties::{
    ACTIVATION_CHOICES, AttendantPropertiesState, OriginalValues, PickDirection, PropertyField,
    TRIGGER_CHOICES, attendant_properties_scope, attendant_properties_slot,
    attendant_seed_template_scope, pick_activation, pick_trigger,
};
pub use crate::report::AttendantReport;
pub use crate::report_picker::{
    AttendantReportPickerState, attendant_report_picker_scope, attendant_report_picker_slot,
};

#[cfg(test)]
mod tests;

/// The placeholder an attendant's seed template uses to refer to its last report.
pub const PRIOR_REPORT_PLACEHOLDER: &str = "<prior report>";

/// The seed text a fresh attendant starts with when the user has not written one.
///
/// Framed as a hypothesis to check rather than a conclusion to trust, so a
/// re-run treats the previous report as something to confirm or refute.
#[must_use]
pub fn default_seed_template() -> String {
    format!(
        "Your previous run reported: {PRIOR_REPORT_PLACEHOLDER}. \
         Confirm or refute this against the current code."
    )
}
