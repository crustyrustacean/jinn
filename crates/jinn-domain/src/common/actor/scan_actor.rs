//! Shared helpers for cwd-driven scan actors.

use std::path::{Path, PathBuf};

use crate::common::state::State;
use crate::protocol::SessionId;

/// Marker type for actors that don't use direct messages.
///
/// Use as `type Message = NoDirectMsg;` in the `Actor` impl.
pub enum NoDirectMsg {}

/// The cwd sentinel from [`crate::feat::session::chat_session::SessionCore::default`].
///
/// While a session's cwd is this value, a lifecycle setup script is pending and
/// the real cwd is unknown.
const PENDING_CWD: &str = ".";

/// Resolves a session's cwd for a cwd-driven scan, gating out pending setups.
///
/// Returns `None` if the session is not present in state (it may have been
/// closed concurrently) or if its cwd is still the `"."` sentinel — meaning a
/// lifecycle setup is pending and the real cwd is unknown. Returns
/// `Some(cwd)` otherwise.
///
/// This is the shared entry point for every scan actor's event handler: callers
/// fire a scan only when this returns `Some`, which prevents junk scans before
/// `SessionSetupCompleted` resolves the real cwd.
pub fn scan_cwd_for_session(state: &State, session_id: &SessionId) -> Option<PathBuf> {
    let guard = state.read();
    let session = guard.try_session(session_id)?;
    let cwd = session.cwd();
    if cwd == Path::new(PENDING_CWD) {
        None
    } else {
        Some(cwd.to_path_buf())
    }
}
