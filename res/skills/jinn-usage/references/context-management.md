# Context Management

jinn gives you direct control over what enters the LLM's context window. Every
chat entry (user message, assistant reply, tool call, tool result) can be
pinned, excluded, or reset individually — and background workers prune
automatically. All bindings below are normal mode unless stated.

## Selecting an entry

- `j` / `k` move the selection down / up the chat history.
- Jumps: `]u`/`[u` (user messages), `]p`/`[p` (pinned), `]c`/`[c`
  (compaction summaries), `]s`/`[s` (Sources annotations), `gg`/`G` (top/bottom).
  Each `]` jumps forward and its `[` partner jumps backward; the Sources jump
  clamps at both ends rather than wrapping.

## Pinning an entry

Pinned entries are **always** included in the LLM context — auto-prune workers
and compaction skip them. Pin durable facts: a plan, API contract, naming
decision, or correction you want enforced for the rest of the session.

1. Focus the chat history (normal mode) and select the entry with `j`/`k`
   (or a `]`/`[` jump).
2. Press `p` to pin it.

Pins apply to a whole **tool loop as a unit** — pinning a tool call also pins
its matching tool result, so the pair never desyncs in context.

### Choosing where a pin sits

Position matters: entries nearer the end of the context have more influence
on the next reply, and pinning mid-history can change how the model reads
later messages. To reposition:

1. Focus the sidebar with `<c-l>`, then `J`/`K` until the **Pins** section is
   highlighted (or jump straight there and browse).
2. `j`/`k` to select the pin, then:
   - `t` — move to the **top** of context (strongest "always in view" signal).
   - `b` — move to the **bottom** of context (highest recency weight, closest
     to the newest messages).
   - `r` — move to a **relative** position (near where the entry sits in
     history).
   - `m` — **cycle** through positions.
3. `<enter>` (or `<esc>`/`<c-h>`) leaves the sidebar and returns to the chat,
   restoring your history position.

**Effect of each position:**
- *Top*: reads like a standing rule; suits project-wide constraints, but is
  far from the current turn.
- *Bottom*: closest to the newest messages, so it has the most weight in the
  next reply; suits "do X now" instructions. Moves up as the conversation
  grows.
- *Relative*: stays near where the entry sits in history; suits comments that
  explain a specific block of history.

### Jumping among pins

`]p` / `[p` snap the selection to the next / previous pinned entry without
opening the sidebar.

## Excluding an entry

- `x` toggles the selected entry **out of** the LLM context ("ignore"). Useful
  when you sent a message you do not want the model to see: the text stays
  visible in the TUI but does not enter context.
- `r` resets the entry to default handling (auto-prune workers decide again).
- Excluded entries stay visible in the TUI (collapsed, dimmed by default);
  `h` toggles visibility of the collapsed blocks.
- Like pinning, `x` applies to a tool call + result as one unit.

## Isolate a single exchange: `gci`

To act on **only** one entry — a plan, or a correction — select it and press
`gci` (a sequence: `g`, `c`, `i`). jinn force-includes that entry's tool loop
and force-excludes every other non-pinned entry; pins are untouched. The
context then contains only that exchange. Undo per-entry with `x`/`r`, or
start a new session (`n`).

## Background pruning and compaction

jinn prunes stale detail automatically so context stays small and prefix-cache
friendly; compaction (summarizing) exists only as a backstop and should almost
never fire during coding. Related tuning lives in `jinn.toml`:

- `[context_curation.auto_prune.*]` — per-strategy rules (stale reads, repeated
  edits, old tool output, trivial assistant chatter, regex rules...), each with
  `enabled`, `min_age`, and per-strategy thresholds.
- `[context_curation.auto_prune] accumulation_threshold_tokens` — prune
  context-mutations are batched until this token budget accumulates, protecting
  prefix-cache hits.
- `[context_curation.compaction]` — `threshold` (usage fraction that triggers
  compaction), `reserve_tokens`, `fallback_context_window`, and an optional
  compaction `model`.

See `configuration.md` for how to edit these and the restart caveat.

## Related bindings

| Key | Action |
| --- | --- |
| `e` | Expand / collapse the selected tool entry |
| `a` | Toggle the audit popup for the selected entry |
| `y` | Yank the selected entry's raw content |
| `gcp` | Set the pruner accumulation threshold interactively |

## The audit popup (`a`)

`a` on a selected entry opens a read-only audit popup describing how that
entry was produced. Every provider-produced entry carries the concrete
model that generated it, so the popup shows a `Model:` line; when the
entry's model differs from the model the session requested at dispatch
time (OpenRouter's `auto` routing strategy is the usual cause), an
indented `via <requested>` line renders beneath it.

## Pins in the sidebar

The sidebar **Pins** section shows one row per tool **result** of a pinned
tool loop (plus one row per tool call still missing its result, and the
loop's opener for a standalone pinned entry), so pinning a saved plan or a
skill load produces one row, not one row per entry. Pin state still applies
to the whole loop.
