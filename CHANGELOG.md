## (development) v0.104.0

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
