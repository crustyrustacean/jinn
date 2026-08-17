---
name: jinn-plugin
description: Author, build, and install jinn plugins — WASM components hosted in-process by jinn. Use when the user wants to create a plugin, scaffold a plugin project, build a plugin to .wasm, install/uninstall a plugin, or write plugin code against the jinn plugin wire contract (handshake, event subscriptions, contributions like SetThemeEntries).
---

# jinn Plugin Authoring

Plugins are WASM components (`wasm32-wasip2` target) hosted in-process by jinn.
They speak NDJSON over stdin/stdout: one JSON envelope per line. Plugins are
**async producers** — they push contributions (e.g. theme definitions) that jinn
caches in `AppState`. Nothing plugin-side is ever on the sync render path.

---

## The loop

```
jinn plugin new my-plugin          # scaffold (current dir)
cd my-plugin
cargo build --target wasm32-wasip2 --release
jinn plugin install target/wasm32-wasip2/release/my_plugin.wasm
# restart jinn — plugins spawn at app start
```

---

## Wire contract

Every line is `{"v":1,"seq":N,"ts":...,"msg":{...}}` (serde-tagged `"type"`).
v1 message set:

- Guest → host: `Hello` (protocol version + event subscriptions), `SetThemeEntries`
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
4. Guest then does its work — e.g. scan granted dirs, parse, push
   `SetThemeEntries`
5. Guest `main` returning = guest end; jinn marks the plugin `Dead`

## Capabilities (grants)

Declared per-plugin in `jinn.toml`:

```toml
[[plugin]]
name = "my-plugin"
wasm = "my-plugin.wasm"   # relative to <data_dir>/plugins/
http = false              # wasi:http network access
grants = [ { path = "<config_dir>/themes" } ]   # read-only preopen
```

- `grants` = filesystem preopens; no grant means no filesystem access at all
- Every plugin automatically gets `<data_dir>/plugins/<name>/` as writable scratch
- `<config_dir>` / `<data_dir>` variables are substituted at spawn
- Guests cannot spawn processes; env access is host-controlled

## SDK

`jinn plugin sdk` downloads the SDK crates to
`<data_dir>/plugin-sdks/<version>/`. Point the scaffold's `Cargo.toml` at them
via the printed `path = "..."` lines if not using the registry. The SDK gives
typed wire types (`Envelope`, `Hello`, `Welcome`, `SetThemeEntries`,
`ThemeDef`) and helpers for the handshake — always prefer it over hand-rolling
JSON.

## Gotchas

- **Restart required.** Installing does not spawn anything; plugins activate at
  next jinn start. There is no hot-reload.
- **Stdout is protocol.** `println!` in a guest corrupts the stream — write
  envelopes, nothing else. Debug via stderr (captured to a ring, not shown
  inline).
- **Theme colors** accept ANSI name, ANSI code (u8), hex `#rrggbb`, or
  `[r, g, b]`.
- **Flooding** is debounced host-side: bursts of `SetThemeEntries` collapse;
  do not rely on every message landing as a separate state write.
- **Validation** happens host-side on every inbound message; malformed lines
  are dropped (status stays Running), never crash jinn.

## Testing

- Guest crates test TOML parsing/scanning logic as plain Rust unit tests
- Host-side contract tests live in `jinn-plugin-api` (schema drift) and
  `jinn-plugin` (framing); reuse them, don't reimplement
- For e2e, build the real guest and run jinn; a dead/missing plugin must
  degrade gracefully (defaults only), never block startup
