## 2026-08-29 Unreleased

- Add the `interactive_term` tools — run and drive interactive terminal programs (vim, psql, ssh, htop, REPLs) from the agent:
  - `interactive_term` spawns the program in a pseudo-terminal (with its own controlling tty, unlike the deliberately tty-less `bash` children) and returns the rendered screen once output settles.
  - `interactive_term_send` types text and presses named keys (`enter`, `ctrl+c`, arrows, ...); call with no inputs to re-sync the screen.
  - `interactive_term_kill` terminates the whole process group and returns the final screen, transcript tail, and exit code (safe on already-exited sessions).
  - Sessions persist across tool calls; calls block only for the settle window (default quiet 400ms, cap 3s — configurable via `[interactive_term]` in `jinn.toml`).
  - While a call runs, heartbeat events keep the stall watchdog from retrying long interactive work.
- Add the terminal tab to the `<Tab>` cycle: view mode is passive; `i` takes control (keys forward to the program); the handback key (default `<c-g>`, configurable) returns control to the agent and steers the captured screen to the model — drained into an in-progress turn, or dispatched immediately when idle.

## 2026-08-27 v0.112.1

- Add better Windows support (contributor: Jeff Mitchell <crusty.rustacean@gmail.com>)
  - Fix Kitty keyboard startup failure
  - Git Bash resolution
  - Browser version probe

## 2026-08-27 v0.112.0

- Add keybind `gci` in normal mode to "isolate" the selected chat entry.
  - The motivating use-case for this is planning -> isolate the approved plan -> implement with fresh context to maintain lifecycle.
- Changed context assembly order for system prompt.

## 2026-08-27 v0.111.0

- Add user filter for Discord bot usage. The bot will only respond to users listed in the TOML.
  - This is a minimal implementation, there is no role filtering or allow-all.

```toml
# jinn.toml
[discord]
enabled = true              # Whether the Discord bot is active.
guild_id = "123"            # Discord guild (server) ID the bot operates in.
forum_channel = "456"       # Forum channel where the bot creates session threads.
authorized_users = ["789"]  # Users allowed to interact with the bot.
```

## 2026-08-26 v0.110.0

- Move citation tracking into plugin.
  - Add support for tracking citations via ZAI `web-search-prime` MCP tool
  - Add support for tracking citations using `web-search` + `web-fetch` tool
  - Add support for tracking citations using `openrouter:web-search` tool
- Disabling an MCP server now removes it's associated tools from the session tool listing.
- Add plugin status screen under `<leader>sP`.
- Cache indicator is now colorized based on cache hit rate.
  - <90%: red
  - 90%-94%: yellow
  - 95%+: green
- Configuration update for to allow adjustment of default tools + skills + MCP servers for sessions.
- MCP servers can now use custom headers.

### TOML Config Update

The behavior of tools and skills is "enable everything" by default for every session. Configuration options have been added to disable specific tools and skills. These apply to all new sessions.

All MCP servers are disabled by default. There is now an additional `auto_enable` flag on the MCP server configuration block which will start the MCP server on every new session.

```toml
# list tools to be disabled by default
disabled_tools = ["web-search", "web-fetch", "openrouter:web_search"]

# list skills to be disabled by default
disabled_skills = ["foo", "bar"]

# MCP autostart example
[mcp_server.parallel-search]
transport = "remote_http"
url = "https://search.parallel.ai/mcp"
# Defaults to false, but when true: MCP server will start up on every session
auto_enable = true
```

### MCP Custom Headers

Many remote MCP servers require an API key. `jinn` now supports adding custom headers with environment variable token substitution.

```toml
[mcp_server.web-search-prime]
transport = "remote_http"
url = "https://api.z.ai/api/mcp/web_search_prime/mcp"

# Headers specified in another block
[mcp_server.web-search-prime.headers]
# `${FOO}` expands to an environment variable named FOO
Authorization = "Bearer ${ZAI_API_KEY}"
```

## 2026-08-25 v0.109.0

- Group tool calls atomically to prevent malformed chat history construction.
  - A side-effect of this change is that pins and context exclusion now operate on multiple entries as a group.

## 2026-08-20 v0.108.4

- Add filters to prevent invalid message sequencing being sent to providers.

## 2026-08-20 v0.108.3

- Switched tools + skills backing data to a `BTreeMap` to prevent future cache issues.

## 2026-08-19 v0.108.2

- Skill construction no longer busts cache.

## 2026-08-18 v0.108.1

- Tool construction no longer busts cache.

## 2026-08-18 v0.108.0

- `jinn install` now installs builtin plugins
- Removed hashline implementation.
- Updated read/write/edit tools to use more common harness schemas and tool descriptions.

## 2026-08-17 v0.107.0

- `jinn` will immediately exit on startup if `jinn.toml` or `providers.toml` is malformed.
- Yanking from chat history tool results will now return a complete JSON object instead of the displayed text. This is to enable piping into `jq` for processing.
- Bugfix: Tool output should no longer leak and overwrite the TUI.
- Experimental plugin system added. Plugins are written in Rust and compiled to WASM. See the jinn-plugin skill for more information.

## 2026-08-15 v0.106.0

- Add `[[providers.model_info]]` tables to `providers.toml`: per-model `context_length`, `input_modalities`, and `extra_body` overrides. Hand-authored values take precedence over API-discovered data and models.dev. Models that are only listed in `providers.toml` (never discovered) now appear in the model cache, so the status bar, compaction gate, and attachment gate resolve them; `input_modalities = ["text", "image"]` marks a local model vision-capable.
- Update learning-tutor persona.
- Skill preview rendering now uses a shared cache across all sessions.
- **Breaking:** `providers.toml` providers are now map-keyed tables (`[providers.<name>]`) instead of `[[providers]]` array-of-tables.

### Map-Keyed Providers

Nested tables are now part of the key path, so a nested table's provider is self-describing: `[providers.zai.extra_body]` and `[[providers.zai.model_info]]` unambiguously belong to zai instead of relying on which `[[providers]]` block came before them. Duplicate provider names are now rejected by TOML itself, and file order carries no meaning. Two things to know when converting an existing file: dotted names like `llama.cpp` are no longer valid as table keys (rename to something dot-free, e.g. `llamacpp`), and legacy files fail to load with an error naming the new syntax — the conversion is renaming each block header to `[providers.<its name>]` and deleting the `name =` key inside it.

```toml
# example
[providers.llamacpp]
backend = "openai"
requires_key = false
base_url = "http://127.0.0.1:8089/v1"
models = [
    "/path/to/model.gguf",
    "/foo/bar.gguf
]

[[providers.llamacpp.model_info]]
id = "/path/to/model.gguf"
context_length = 96000
input_modalities = ["text", "image"]

# the `bar.gguf` model is not listed, so it gets application defaults (unknown context + text-only modality)
```

## 2026-08-07 v0.105.0

- Add `F` keybind in Normal mode to create a _new_ session from an existing User or Assistant message _without_ existing history. Only the selected message will be present in the new session. The new session is not counted as a fork of the previous session since no history is preserved.
- Add `--dump-requests` debugging flag to get raw output of everything sent to a provider.
- Migrations should no longer fail if interrupted.
- Migrations performance improvement (single transaction).

## 2026-08-03 v0.104.1

- Simpler MCP configuration + added MCP configuration example

## 2026-08-03 v0.104.0

- MCP server sidebar entry only shows when MCP servers are enabled. Also uses consistent padding (1 cell top + 1 cell bottom).
- Uploaded tokens indicator now uses provider-returned value, falling back to local calculation.
- Cached tokens are now displayed (w/hexagon icon) next to uploaded tokens indicator as a percentage.
- Reasoning effort is now displayed upon starting `jinn`.

### OpenRouter Provider Selection

OpenRouter endpoints can now be selected using `<leader>sE` and selection is persisted per session. When using OpenRouter it's recommended to manually select a provider in order to maximize prompt cache pricing. Default behavior is unchanged and uses your OpenRouter account configuration for endpoints.

## 2026-07-27 v0.103.0

- `@` behavior now allows sending a message if the attachment cannot be found. Missing files are highlighted in red.
- `@` popup now scrolls.

## 2026-07-25 v0.102.0

- Fix slow startup performance on `install` and `config` commands.
- Add `--force` flag to `install` command to overwrite existing files.
- Fix positioning of file selection and prompt selection popups to be word-wrap aware. They now appear directly over the cursor instead of above the input box.
- Provide base directory in context for loaded skills to help agent load reference files.
- Add MCP server support.
- "Nag" system defaults changed: 200 chat entries + disabled
- Headed Chrome/Chromium instance restarts on connection lost. This happens if a `web_fetch` or `web_search` request hasn't happened for a while.

## 2026-07-23 v0.101.0

- Add ability to load skills directly via the skill picker using `<c-l>`.
- Skills that have been loaded can no longer be disabled from the skill picker.

### New Feature: Project Record

A new "record" file can be placed at `./agents/RECORD.md` which lists out current high-level facts about the project.

The idea behind the record is that it always gets managed by a human and gets surfaced during every feature change. Humans can easily read and edit the record, and the agent has been instructed to only write approved edits to the record. The agent loads it and so gains an understanding of how things are _supposed_ to work which should (in theory) speed up codebase exploration during planning. It also helps (but doesn't eliminate) the problem of docs drifting from the actual implementation since the record gets referenced regularly.

For example, a fact might be "The database backend is SQLite so the application can be ran easily standalone" and a new feature request might be "Add Postgres support". This pre-existing fact will get surfaced during planning so the developer can be made aware of potential implications of the feature.

The implementation is prompt-based and all relevant planning and analysis prompts have been updated to support managing the record. Note that this takes slightly more tokens since extra work is being performed. But the token cost shouldn't be significant since the agent will already have the relevant code loaded into context during planning and evaluation.

Currently its just a markdown file. As I experiment with using it, it might get changed to a vector search or some other lower-context mechanism.

## 2026-07-21 v0.100.0

- New "nag" system reminds agent to use the `todo_*` tools if they haven't done so after 100 chat entries.
- Add `cargo binstall` support

## 2026-07-21 v0.97.0

- Discord integration now allows selection of lifecycle script
