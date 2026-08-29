## 2026-08-29 Unreleased

- Rework the interactive terminal UI: the terminal **tab** is replaced by a centered, colored **overlay** that updates in realtime (~50ms) even when no tool call is in flight. Toggle it with the global `<M-t>` (active session) or `T` on a selected session in the sidebar, which now marks sessions with a live terminal. The `<Tab>` cycle is back to Dashboard ↔ Normal.
- `interactive_term` sessions are now linked to the chat session: each chat session has at most one terminal (spawning again kills the previous program and reports it), and spawning without a chat session is rejected. The PTY is sized to the overlay's inner rect and spawned in the tool context's working directory.
- Agent key input now supports `f1`–`f12` (previously silently dropped), so programs like htop respond to function keys.

- Add the `interactive_term` tools — run and drive interactive terminal programs (vim, psql, ssh, htop, REPLs) from the agent:
  - `interactive_term` spawns the program in a pseudo-terminal (with its own controlling tty, unlike the deliberately tty-less `bash` children) and returns the rendered screen once output settles.
  - `interactive_term_send` types text and presses named keys (`enter`, `ctrl+c`, arrows, ...); call with no inputs to re-sync the screen.
  - `interactive_term_kill` terminates the whole process group and returns the final screen, transcript tail, and exit code (safe on already-exited sessions).
  - Sessions persist across tool calls; calls block only for the settle window (default quiet 400ms, cap 3s — configurable via `[interactive_term]` in `jinn.toml`).
  - While a call runs, heartbeat events keep the stall watchdog from retrying long interactive work.
- Add the terminal tab to the `<Tab>` cycle: view mode is passive; `i` takes control (keys forward to the program); the handback key (default `<c-g>`, configurable) returns control to the agent and steers the captured screen to the model — drained into an in-progress turn, or dispatched immediately when idle.

## (development; unreleased) v0.113.0

- Add `A` sidebar keybind to archive selected session and all of it's children.
- Add subagents/spawnable subtasks.

### Subagents

Subagents can be spawned using a new `task` tool and will appear in _purple text_ as a child session in the sidebar. The max depth is set to 1 prevent runaway cascading subagents.

Subagent sessions are regular sessions that you can load to view their progress, steer, or cancel whenever desired. Once the subagent session returns to an IDLE state, the last message in the session (regardless of what it is) is sent back to the parent as a tool result. This makes it impossible to introduce a broken program state by manually working with a subagent since it's just a parent session calling a tool and waiting for the result.

## 2026-08-27 v0.112.1
