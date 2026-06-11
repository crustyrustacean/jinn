//! Shutdown commands.

use serde::{Deserialize, Serialize};


/// Proceed with shutdown after actor coordination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProceedWithShutdown {
    /// Actors that completed shutdown successfully.
    pub completed: Vec<String>,
    /// Actors that did not respond before timeout.
    pub timed_out: Vec<String>,
}
