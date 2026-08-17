---
name: jinn-plugin
description: Author, build, and install jinn plugins — WASM components hosted in-process by jinn. Use when the user wants to create a new plugin, scaffold a plugin project, build a plugin to .wasm, install or remove a plugin (`jinn plugin add`/`build`/`install`), or write plugin code against the jinn plugin wire contract (handshake, event subscriptions, typed contributions).
---

# jinn Plugin Authoring

Plugins are WASM components (`wasm32-wasip2` target) hosted in-process by jinn.
They speak NDJSON over stdin/stdout: one JSON envelope per line. Plugins are
**async producers** — they push typed contributions that jinn caches in
`AppState`. Nothing plugin-side is ever on the sync render path.

---

## The loop

```
jinn plugin new my-plugin          # scaffold (current dir)
  # --sdk <path>  → local jinn checkout (plugin develops against uncommitted SDK)
  # --sdk <git-url[@rev]> → any jinn fork/commit
cd my-plugin
jinn plugin add .                  # build + install in one command
# restart jinn — plugins spawn at app start
```

`plugin add` = read manifest → build (embeds manifest) → install with the
declared grants. The separate primitives remain available:

- `jinn plugin build` — wraps `cargo build --release --target wasm32-wasip2`,
  resolves the artifact path from cargo itself (in-workspace crates build to
  the workspace root's `target/`, standalone crates to their own), embeds the
  manifest into the artifact
- `jinn plugin install <wasm>` — copies the payload, writes the config entry,
  applies embedded manifest grants

**Removing a plugin:** delete its `[plugin.<name>]` table from `jinn.toml`
(or set `enabled = false`) and restart. There is no uninstall command yet.

## Plugin manifest

A plugin declares its needs in its `Cargo.toml`:

```toml
[package.metadata.jinn]           # required — marks the crate as a jinn plugin
name = "my-plugin"                # optional install name; defaults to the crate name
grants = [                        # filesystem directories the guest may access
  "<config_dir>/my-plugin",       # read-only preopen of a user config area
  "<plugin_data_dir>:w",          # writable scratch dir, persistent across runs
]
http = false                      # true enables outbound wasi:http requests
```

`plugin build` hard-errors without the section (a plain Rust crate is not a
plugin) and embeds it into the artifact as a `jinn_manifest` custom section —
artifacts are self-contained. `plugin add`/`install` extract it and auto-apply
the declared grants, printing each one. Flags override per install:
`--grant '<path>'` replaces the grant list; `--http`/`--no-http` replace the
http setting; `--name` replaces the name. Install fails hard on a `.wasm`
with no embedded manifest (rebuild it with a current jinn).

---

## Wire contract

Every line is `{"v":1,"seq":N,"ts":...,"msg":{...}}` (serde-tagged `"type"`).
Message set:

- Guest → host: `Hello` (protocol version + event subscriptions), plus
  contribution messages — one typed struct per kind of data a plugin provides
- Host → guest: `Welcome` (plugin id, granted dirs, config)
- Unknown variants deserialize to `Unknown` on both ends — old jinn + new
  plugin (or vice versa) never crashes, messages are ignored

The contract is frozen in `plugin-api.schema.json`; wire types live in the
`jinn-plugin-api` crate. Add new message types **additively** — never reorder
or repurpose existing ones.

## Handshake

1. jinn spawns the guest at app start
2. Guest writes `Hello { "protocol_version": 1, "subscriptions": [] }` as its
   **first line**
3. Host replies `Welcome { "id": ..., "granted_dirs": [...], "config": ... }`
4. Guest then does its work — read granted directories, fetch over http (if
   granted), or react to subscribed events, and push contributions whenever
   its data changes
5. Guest `main` returning = guest end; jinn marks the plugin `Dead`

## Capabilities (grants)

Overridable in `jinn.toml` after install:

```toml
[plugin.my-plugin]                # table name IS the plugin identity
wasm = "my-plugin.wasm"           # payload location; relative to <data_dir>/plugins/
http = true                       # wasi:http network access
enabled = true                    # false skips spawning without uninstalling
grants = [                        # replaces the manifest-declared grant list
  "<config_dir>/my-plugin",       # read-only preopen; append :w for writable
]
```

- `grants` = filesystem preopens; no grant means no filesystem access at all
- Nothing is granted implicitly — persistence needs `"<plugin_data_dir>:w"`
- Guests cannot spawn processes; env access is host-controlled

Substitution tokens usable in any grant path:

| Token | Meaning |
| --- | --- |
| `<config_dir>` | jinn's user config directory (e.g. `~/.config/jinn`) |
| `<data_dir>` | jinn's user data directory (e.g. `~/.local/share/jinn`) |
| `<plugin_data_dir>` | this plugin's scratch dir: `<data_dir>/plugins/<name>` |

Tokens are substituted at spawn; an unknown token is a hard error (the plugin
is marked `Dead` rather than spawning with surprise access).

## SDK

Guest code depends on two crates, both resolved from the jinn repo via git
(never crates.io):

```toml
[dependencies]
jinn-plugin-api = { git = "..." }   # typed wire types — Envelope, Hello, Welcome, contributions
jinn-plugin-sdk = { git = "..." }   # handshake + I/O helpers built on the api crate
```

`jinn plugin new` scaffolds these for you. The SDK gives typed wire types and
helpers for the handshake — always prefer it over hand-rolling JSON.

## Gotchas

- **Restart required.** Installing does not spawn anything; plugins activate at
  next jinn start. There is no hot-reload.
- **Stdout is protocol.** `println!` in a guest corrupts the stream — write
  envelopes, nothing else. Debug via stderr (captured to a ring, not shown
  inline).
- **Flooding** is debounced host-side: bursts of identical contributions
  collapse; do not rely on every message landing as a separate state write.
- **Validation** happens host-side on every inbound message; malformed lines
  are dropped (status stays Running), never crash jinn.

## Testing

- Guest crates test their parsing/scanning logic as plain Rust unit tests
- Host-side contract tests live in `jinn-plugin-api` (schema drift) and
  `jinn-plugin` (framing); reuse them, don't reimplement
- For e2e, build the real guest and run jinn; a dead/missing plugin must
  degrade gracefully (defaults only), never block startup
