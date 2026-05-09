//! Dashboard intent validators.
//!
//! Validators for dashboard navigation intents.
//! All are infallible — they always succeed.

use nullslop_component::AppState;

/// Validates the DashboardSelectDown intent.
pub fn validate_dashboard_select_down(_state: &AppState) {}

/// Validates the DashboardSelectUp intent.
pub fn validate_dashboard_select_up(_state: &AppState) {}

/// Validates the DashboardSelectFirst intent.
pub fn validate_dashboard_select_first(_state: &AppState) {}

/// Validates the DashboardSelectLast intent.
pub fn validate_dashboard_select_last(_state: &AppState) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn validate_dashboard_select_down_always_succeeds() {
        let state = AppState::default();
        validate_dashboard_select_down(&state);
    }

    #[rstest::rstest]
    fn validate_dashboard_select_up_always_succeeds() {
        let state = AppState::default();
        validate_dashboard_select_up(&state);
    }

    #[rstest::rstest]
    fn validate_dashboard_select_first_always_succeeds() {
        let state = AppState::default();
        validate_dashboard_select_first(&state);
    }

    #[rstest::rstest]
    fn validate_dashboard_select_last_always_succeeds() {
        let state = AppState::default();
        validate_dashboard_select_last(&state);
    }
}
