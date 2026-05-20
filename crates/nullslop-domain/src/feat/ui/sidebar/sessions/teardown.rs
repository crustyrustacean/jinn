//! Session teardown handler.

use crate::common::app_state::AppState;
use crate::feat::ui::sidebar::sessions::close::validate_session_close;
use crate::feat::ui::sidebar::sessions::state::sorted_open_sessions;

/// Handles `SidebarSessionTeardown` — re-runs teardown without closing the session.
///
/// Validates that the close can proceed, looks up the selected session's
/// teardown command, and emits `RunSessionTeardown` with `close_on_success: false`.
/// If the session has no teardown command, this is a no-op.
///
/// # Panics
/// Panics if `sessions_section.selected_index` is `None`.
pub fn handle_session_teardown(state: &mut AppState) -> crate::protocol::IntentResult {
    use crate::feat::session_lifecycle::command_template::CommandTemplate;

    // Validate — same preconditions as session close.
    if validate_session_close(state).is_err() {
        return crate::protocol::IntentResult::empty();
    }

    let index = state.frontend.sessions_section.selected_index.unwrap();
    let sessions = sorted_open_sessions(state);
    let target_id = sessions[index].id.clone();

    // Look up teardown command for the session.
    let (teardown_command, lifecycle_args) = {
        let session = state.session.sessions().get(&target_id);
        let Some(session) = session else {
            return crate::protocol::IntentResult::empty();
        };
        let lifecycle_name = session.lifecycle_name().map(String::from);
        let args = session.lifecycle_args().to_vec();
        let teardown = lifecycle_name.as_deref().and_then(|name| {
            state
                .frontend
                .preferences
                .session_lifecycles
                .iter()
                .find(|l| l.name == name)
                .and_then(|l| l.teardown_command.clone())
        });
        (teardown, args)
    };

    let Some(ref teardown_cmd) = teardown_command else {
        return crate::protocol::IntentResult::empty();
    };

    let template = CommandTemplate::parse(teardown_cmd);
    let rendered = if lifecycle_args.is_empty() {
        teardown_cmd.to_owned()
    } else {
        template.render(&lifecycle_args)
    };

    crate::protocol::IntentResult::with_commands(vec![
        crate::protocol::Command::RunSessionTeardown(
            crate::feat::session_lifecycle::protocol::command::RunSessionTeardown {
                session_id: target_id,
                command: rendered,
                args: lifecycle_args,
                close_on_success: false,
            },
        ),
    ])
}
