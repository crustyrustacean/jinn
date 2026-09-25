# Phase 6 Cross-Crate Behavior Coverage

The migration already added focused observable tests at each real crossing
boundary. No duplicate tests are added because each requested behavior is
already pinned by a test whose name states the observable outcome.

| Behavior | Existing test boundary |
| --- | --- |
| Loaded session is fully initialized before completion publication | `jinn-session-store/src/session_store_actor_tests.rs::load_completed_is_published_after_session_is_fully_initialized` |
| Archive failure leaves the live session active | `jinn-session-store/src/session_store_actor_tests.rs::archive_write_failure_leaves_session_live_and_active` |
| Successful archive removes the live session | `jinn-session-store/src/session_store_actor_tests.rs::archive_session_removes_session_from_state` |
| Lifecycle cwd write and event | `jinn-session-lifecycle/src/session_lifecycle_actor_tests.rs::{set_session_cwd_updates_session_working_directory,set_session_cwd_publishes_session_cwd_changed}` |
| Lifecycle setup and close/archive command boundaries | `jinn-session-lifecycle/src/session_lifecycle_actor_tests.rs::{builtin_setup_completes_session_busy_state,builtin_setup_publishes_setup_completion_event,close_session_without_pending_teardown_publishes_archive_session}` |
| Sidebar reconciles through `SessionRemoved` and clamps cursor | `tests/slices/sidebar.rs::session_closed_crosses_to_sidebar_and_clamps_cursor` plus sidebar actor reconciliation tests |
| Sidebar promotion and visual-parent repair | `jinn-sidebar/src/sections/sessions_tests.rs::{close_root_session_promotes_children_to_roots,update_visual_parents_on_removal_reparents_only_children_of_removed_session}` |
| MCP runtime clears on close | `jinn-mcp-slice/src/coordinator.rs::session_closed_clears_runtime_data` |
| MCP runtime clears on archive | `jinn-mcp-slice/src/coordinator.rs::session_archived_clears_runtime_data` |
| MCP runtime clears on teardown | `jinn-mcp-slice/src/coordinator.rs::session_teardown_finished_clears_runtime_data` |
| MCP runtime clears before load reconciliation | `jinn-mcp-slice/src/coordinator.rs::session_load_completed_clears_stale_runtime_data` |
| MCP status/stderr are session-isolated | MCP coordinator runtime-cell isolation tests for status and stderr |
| Tools child construction inherits parent config | `jinn-tools/src/task_tests.rs::{task_spawns_child_linked_and_inheriting,task_child_inherits_parent_project}` |
| Token actor handles completed loads | `jinn-token-count/src/count_actor.rs` load-completion test |
| Context assembly consumes the state projection | `jinn-context-assembly/src/service.rs::ask_returns_assembled_prompt` and assembly-input tests |
| Picker rows and load command | `tests/slices/picker.rs::{open_session_picker_enters_picker_scope,confirm_session_picker_begins_loading_the_selected_session}` |
| Discord session routing | `tests/slices/discord.rs` keymap/group composition tests and gateway projection tests |
| Watchdog retry updates real session history | `tests/slices/watchdog.rs::silent_stream_trips_the_stall_watchdog_and_the_marker_lands_in_history` |
| Snapshot revisions and stale saves | state capture test and `jinn-session-store/src/sqlite_tests.rs` stale-save tests |
| Fork metadata/history/tool permissions | `jinn-session-store/src/sqlite_tests.rs` fork, ordinal, origin, and profile-policy tests |
| Flat JSON and legacy compatibility | exact JSON-shape and legacy-load tests in `jinn-session-store/src/sqlite_tests.rs` |
| SQLite write/load transaction behavior | transaction rollback and coherent-read tests in `jinn-session-store/src/sqlite_tests.rs` |
