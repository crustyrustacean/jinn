## 2026-08-25 v0.108.5 (unreleased)

- History editor API: all chat-history writes route through a single editor that treats assistant tool-call/result loops as atomic chunks; context toggles and pins apply to whole loops with pin > user > worker precedence, preventing invalid message sequences from ever reaching providers.
- A tripwire validator at the converter output strips any residual invalid tool sequence (legacy persisted sessions, future bugs) instead of sending it.

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
