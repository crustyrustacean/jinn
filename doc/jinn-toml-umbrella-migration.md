# Migrating `jinn.toml` to slice-owned config sections

**This is a hard break. jinn will not read your existing keys.**

Every key in `jinn.toml` has moved under an umbrella named for the slice
that owns it. jinn does not translate old keys, does not warn about them,
and does not migrate them. After upgrading, every section reads its
default and jinn behaves like a fresh install until you edit the file.

## Why

A single flat `jinn.toml` had one struct modelling every subsystem's
config, which meant adding a key anywhere was an edit to that struct, and
the kernel crate had to depend on every slice whose config it named. Each
section now lives under its owner's name, so the file shows who owns what
and a new subsystem ships its own section without touching shared code.

## The full key table

Left column is the old spelling, right column is what jinn reads now.
Keys not listed here are unchanged.

| Old | New |
| --- | --- |
| `[auto_prune]`, `[[auto_prune.regex.rules]]` | `[context_curation.auto_prune]`, `[[context_curation.auto_prune.regex.rules]]` |
| `[compaction]` | `[context_curation.compaction]` |
| `[request_retry]` | `[context_curation.request_retry]` |
| `[mcp_server.<name>]` | `[mcp.<name>]` |
| `[[projects]]` | `[[project.projects]]` |
| `[[global_command_policy]]` | `[[project.global_command_policy]]` |
| `[[session_lifecycle]]` | `[[session_lifecycle.lifecycle]]` |
| `[stall_watchdog]` | `[watchdog.stall]` |
| `[tool_call_watchdog]` | `[watchdog.tool_call]` |
| `[minimap]` | `[ui.minimap]` |
| `[cwd_selector]` | `[ui.cwd_selector]` |
| `[openrouter_web_search]` | `[provider.web_search]` |
| `[interactive_term]` | `[term]` |
| `tool_entry_max_lines` (top level) | `chat_log.tool_entry_max_lines` |
| `min_collapse_count` (top level) | `chat_log.min_collapse_count` |
| `disabled_tools` (top level) | `tools.disabled` |
| `disabled_skills` (top level) | `skills.disabled` |
| `max_tool_output_lines` (top level) | `tools.max_output_lines` |
| `max_tool_output_bytes` (top level) | `tools.max_output_bytes` |
| `tool_default_timeout_secs` (top level) | `tools.default_timeout_secs` |

## The moves that are a pure rename

Most of the table is a header rename. For the scalar keys that moved
under an umbrella table, add the table and indent nothing — TOML tables
nest by prefix, so this:

```toml
# before
tool_entry_max_lines = 12
tool_default_timeout_secs = 300

# after
[chat_log]
tool_entry_max_lines = 12

[tools]
default_timeout_secs = 300
```

A mechanical `sed` over your file handles the header renames:

```sh
# Table and list headers
sed -i \
  -e 's/^\[\[auto_prune\.regex\.rules\]\]/[[context_curation.auto_prune.regex.rules]]/' \
  -e 's/^\[auto_prune\]/[context_curation.auto_prune]/' \
  -e 's/^\[compaction\]/[context_curation.compaction]/' \
  -e 's/^\[request_retry\]/[context_curation.request_retry]/' \
  -e 's/^\[mcp_server\./[mcp./' \
  -e 's/^\[\[projects\]\]/[[project.projects]]/' \
  -e 's/^\[\[global_command_policy\]\]/[[project.global_command_policy]]/' \
  -e 's/^\[\[session_lifecycle\]\]/[[session_lifecycle.lifecycle]]/' \
  -e 's/^\[stall_watchdog\]/[watchdog.stall]/' \
  -e 's/^\[tool_call_watchdog\]/[watchdog.tool_call]/' \
  -e 's/^\[minimap\]/[ui.minimap]/' \
  -e 's/^\[cwd_selector\]/[ui.cwd_selector]/' \
  -e 's/^\[openrouter_web_search\]/[provider.web_search]/' \
  -e 's/^\[interactive_term\]/[term]/' \
  "$HOME/.config/jinn/jinn.toml"
```

**Leave `[[session_lifecycle]]` field names alone.** They were already
`setup_command` and `teardown_command` and still are; only the array
header moved.

## The moves that need hand-editing

The scalars that moved *under* a table cannot be handled by a header
rename, because they were top-level keys and now live inside a section.
Move each one by hand, creating its table if the file has none:

| Key | Put it under | In a new table named |
| --- | --- | --- |
| `tool_entry_max_lines` | `chat_log` | `[chat_log]` |
| `min_collapse_count` | `chat_log` | `[chat_log]` |
| `disabled_tools` | `tools` | `[tools]` |
| `disabled_skills` | `skills` | `[skills]` |
| `max_tool_output_lines` | `tools` | `[tools]` |
| `max_tool_output_bytes` | `tools` | `[tools]` |
| `tool_default_timeout_secs` | `tools` | `[tools]` |

TOML has no dotted-key syntax for adding a parent to an existing
assignment, so this one is genuinely manual. A worked example:

```toml
# before — three top-level keys
disabled_tools = ["web-search"]
tool_default_timeout_secs = 300
min_collapse_count = 3

# after
[tools]
disabled = ["web-search"]
default_timeout_secs = 300

[chat_log]
min_collapse_count = 3
```

Note `disabled_tools` → `tools.disabled` and `disabled_skills` →
`skills.disabled`: the field name changed as well as the location.

## Unknown tables you can delete

These are stale from features that no longer exist. jinn ignores unknown
keys, so leaving them is harmless, but you can remove them:

```toml
[browser]     # removed feature
[web_fetch]   # removed feature
[web_search]  # superseded by [provider.web_search]
```

`[web_search]` is the one to watch: it looks related to the current web
search config but is not read. The live section is `[provider.web_search]`.

## What about comments?

They are preserved. jinn patches the document rather than re-serializing
it, so a comment above a key you did not touch survives every write. A
comment above a key you *did* change in the same write may be lost, so
prefer editing one section at a time.

## Verifying

The simplest check is to start jinn. A file that still uses old keys does
not fail to parse — it parses, and every old section reads as its default.
So a silent revert to defaults is the symptom to look for, not an error.

For a definitive check, look for the sections themselves:

```sh
grep -E '^\[(context_curation|project|mcp|ui|watchdog|provider|tools|skills|chat_log|term)' \
  "$HOME/.config/jinn/jinn.toml"
```

To start clean instead of migrating, move the file aside and let jinn
write a fresh one from its built-in template:

```sh
mv "$HOME/.config/jinn/jinn.toml" "$HOME/.config/jinn/jinn.toml.bak"
```

## A note on formatting

The first write jinn makes to a migrated file may reformat a few things.
`[[projects ]]` — with a space before the bracket — is legal TOML for the
same key, and jinn normalizes it to the canonical `[[project.projects]]`
when it next touches that section. A table written in inline form
(`project = { projects = [...] }`) is likewise expanded into header form.
Both are cosmetic; neither changes what a key means.
