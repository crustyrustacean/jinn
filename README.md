# jinn

A TUI agent harness with multi-session support and Vim-style keybinds.

## Major Features

- Run any number of concurrent sessions, with live preview
- Which-key style keybind system with help popup
- Quickly navigate and change things via Telescope-inspired picker:
  - Select model/provider
  - Enable/disable skills (with preview) and tools
- Context management
  - Background workers continually manage the context while sessions are in-progress. Changes are buffered (configurable) to take advantage of prefix cache pricing.
  - Individual chat entries can be added to or removed from context using `x`
  - Pin messages with `p` to keep them in context indefinitely
- Fork a new session from any message by hitting `f`
- Agent-managed task list with progress display.
- Customizable personas
- Standard agent harness-y things like `AGENTS.md`, `~/.agents` skill discovery, custom prompts (including project-specific for all of these)

## Installation

Currently only Arch Linux package + build from source are supported for installation.

### Arch Linux

```sh
git clone https://github.com/jayson-lennon/jinn.git
cd jinn
makepkg -si
```

### Build from source

#### Requirements

- Rust toolchain (stable)
- SQLite (`sqlite`)
- `clang`
- `gcc-libs`
- [`just`](https://github.com/casey/just)

```sh
git clone https://github.com/jayson-lennon/jinn.git
cd jinn
cargo build --release
just install-defaults
```

The binary will be at `target/release/jinn` and you'll need to add it to your `$PATH`. The `just install-defaults` command will copy all of the required themes/prompts/plugins/etc to your `~/.config/jinn` directory.

## Usage

```sh
jinn
```

Note that each part of the UI has it's own set of keybinds. Make use of `?` to display which keybinds are available in any given scope (or `F1` if you are in a text input box).

### Creating new sessions

There are multiple ways to create new sessions:

- `/new` in the chat input
- `<M-s>n` (alt+s)n

### Message Queueing

`jinn` supports multiple message queues based on the current session state:

- Session is idle -> message always send immediately
- Session is working:
  - QUEUE mode -> Messages enter a buffer and will be flushed (one at a time) after the agent is done working. Use this mode if you want to wait for the model to finish before they get the next message (like "Please double-check your work").
  - STEER mode (default) -> Messages enter a buffer and will be flushed (all at once) in between tool calls. This allows you to "steer" the model in the middle of it's work.

### Navigation

Navigating between interface elements uses directional keybinds based on spatial position:

- `<c-h>` -> focus left
- `<c-l>` -> focus right
- `<c-j>` -> focus down
- `<c-k>` -> focus up

Under the default theme, anything colored `yellow` means "has focus".

### Custom Prompts

Prompts are inserted using `#foo` where `foo` is the name of the prompt. You will get a popup with your available prompts as soon as you type `#`. You can use the arrow keys to select a prompt.

Prompts are only expanded when they get sent to the model and will always show up as `#foo` in the chat input and in the history. If you want to edit the contents of a prompt _before_ sending, type `#foo#`. As soon as the second `#` is typed, the prompt will be fully expanded in the chat input and you can edit it before sending (this does _not_ change the on-disk prompt).

## Agentic Coding

The primary usage target for `jinn` is agentic coding, so it comes pre-packaged with prompts to help facilitate this.

To create a new feature or project:

1. Start a new session
2. Type `#plan` (to load the planning prompt) followed by what you want to do. The agent will ask questions to clarify things that are ambiguous, and then eventually propose a plan in the chat.
3. You can continue to refine the plan or push back on certain elements of the plan until it looks good. Once ready, send an `#approve-plan` message to approve the plan.
   - Using the `#approve-plan` prompt causes the agent to write a detailed plan that includes code samples and the reasoning behind particular choices. This helps the implementer avoid taking a shortcut like "Oh, we can just do `foo` instead" because the plan will specifically mention why `foo` was _not_ chosen.
4. Tell the agent to use `phased-task-loop` or `simple-task-loop` skill to implement the plan. Also tell the agent to use whatever other project or language-specific skills you like using in the same prompt.
   - `simple-task-loop` will instruct the agent to implement the plan. Use for smaller features where you anticipate little friction.
   - `phased-task-loop` creates something similar to [ExecPlans](https://developers.openai.com/cookbook/articles/codex_exec_plans) for each phase as it implements. The agent will document how things diverged from the original plan, and then write the ExecPlan for the next phase accordingly. This will use more tokens and takes longer, but has a higher chance of success on more complex features. The ExecPlan gets generated based on the actual in-progress implementation at the start of each phase, so it can account for major changes that weren't anticipated in the initial plan.
   - You don't _have_ to use one of these skills to begin the coding loop, but it's recommended because they include instructions about periodically getting the latest code to reduce merge conflicts, and also how to properly manage the task list. The skills are SCM and language-agnostic and have been testing on `git`, `Fossil`, `Rust`, `Kotlin`, `Android`, and Shell scripts.

5. After implementation is complete, submit a `#gap-analysis` message.
   - Using the `#gap-analysis` prompt tells the agent to confirm that the implementation meets the acceptance criteria. It will produce a table and make recommendations based on anything that was missed.

## Configuration

jinn is configured via the files in the `~/.config/jinn` directory:

- [`jinn.toml`](./crates/jinn-domain/src/feat/preferences_actor/default_jinn.toml) - user preferences (create new one with `jinn config init`)
- [`providers.toml`](./crates/jinn-domain/src/feat/provider_infra/default_providers.toml) - LLM provider configuration
- `themes/` - color themes
- `personas/` - personas
- `prompts/` - custom prompts

## Security

None. Bring your OS's sandboxing features.

- Linux: [bubblewrap](https://github.com/containers/bubblewrap)
- macOS: [App Sandbox](https://developer.apple.com/documentation/xcode/configuring-the-macos-app-sandbox)
- Windows: [Windows Sandbox (WSB)](https://learn.microsoft.com/en-us/windows/security/application-security/application-isolation/windows-sandbox/)

## Major Roadmap Items

- Plugins
  - Plugins are a continual WIP. Various parts of `jinn` are being exposed over time to enable more sophisticated plugins.

## Contributing

Contributions are welcome, but please file an issue first. PRs _without_ a corresponding issue will be closed.

Automated agent issues/submissions are welcome, but please identify as a bot/agent in the issue tracker.

## Shoutouts

Lots of inspiration from other projects went into the design of `jinn`:

- [pi](https://github.com/earendil-works/pi)
- [OpenCode](https://github.com/anomalyco/opencode)
- [Which Key](https://github.com/folke/which-key.nvim)
- [telescope.nvim](https://github.com/nvim-telescope/telescope.nvim)

## License

jinn is licensed under the [GNU Affero General Public License v3.0 (AGPL-3.0)](LICENSE).

This project includes third-party software under separate licenses. See [THIRD_PARTY_LICENSES](THIRD_PARTY_LICENSES) for details.

Copyright 2026 Jayson Lennon.
