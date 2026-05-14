//! Commands accepted by the plugin actor.

use serde::{Deserialize, Serialize};

use crate::protocol::CommandMsg;

/// Reload plugin scripts from disk.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("plugin_actor")]
pub struct ReloadScripts;
