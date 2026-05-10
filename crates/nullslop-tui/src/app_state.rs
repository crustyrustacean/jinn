//! Application lifecycle status.

/// The current status of the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppStatus {
    /// Application is initializing.
    #[default]
    Starting,
    /// Application is running and ready.
    Ready,
    /// Application is shutting down.
    ShuttingDown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn default_status_is_starting() {
        // Given a default AppStatus.
        let status = AppStatus::default();

        // When inspecting the default value.
        assert_eq!(status, AppStatus::Starting);

        // Then it is Starting.
    }
}
