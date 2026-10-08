# Datalog reference — the graphs server's full surface

Everything here executes through `mcp__graphs__transact` and
`mcp__graphs__query` with `graph` set to an existing graph name. Each code
block is one call.

## Commands

- `(transact [[:e :a v] ...])` — assert facts. Returns a tx id (unix millis).
- `(retract [[:e :a v]])` — remove a fact. The removal itself is history.
- `(rule [(name ?a ?b) body...])` — register a named derived relation.
  **Memory-only**: gone after a server restart; re-register before use.
- `(query [:find ?vars... :where patterns...])` — read rows.

One command per call. The `graph` argument picks which `.graph` file the
command runs against.

## Facts

`[entity attribute value]`. Types:

| Value | Write it as |
|---|---|
| string | `"hello"` |
| integer / float | `42`, `3.14` |
| boolean | `true` / `false` |
| entity ref | `:alice` (keyword) or `#uuid "550e8400-..."` |
| keyword / tag | `:status/active`, `:red` |
| absent | `nil` |

Keyword entities (`:task/1`, `:step/build`) are deterministic and stable —
the same keyword is the same entity forever. Attributes are plain keywords,
conventionally namespaced `:domain/field`.

## Writing with valid time

Transacts accept an optional map before the facts:

```datalog
(transact {:valid-from "2026-01-01" :valid-to "2026-06-30"} [[:deploy/17 :deploy/status :active]])
```

Omitting the map means valid-from-forever (normal case).

## Reading — patterns and clauses

```datalog
(query [:find ?title ?status
        :where [?e :task/title ?title]
               [?e :task/status ?status]])
```

- `[?e :attr ?v]` — bind any position; two bound positions make it a check.
- Literals check: `[?e :task/status :open]` only matches open tasks.
- `(not [...])` negates a sub-pattern:

```datalog
(query [:find ?t
        :where [?e :task/title ?t]
               (not [?e :task/status :done])])
```

- Rules appear as plain sub-goals: `(reachable ?a ?b)`.

## Time travel

`:as-of N` — transaction time: the graph as of tx counter `N`. The counter is
monotonic and the `transact` tool returns the value it advanced to, so capture
tx ids from writes you may want to rewind to.

```datalog
(query [:find ?status :as-of 3 :where [?e :task/status ?status]])
```

`:valid-at "<date>"` — valid time: the world as it was on that date. Default
(no clause) is now; `:any-valid-time` disables the filter.

```datalog
(query [:find ?s :valid-at "2026-05-01" :where [?e :deploy/status ?s]])
```

The two combine — the full audit read:

```datalog
(query [:find ?status :as-of 4 :valid-at :any-valid-time :where [?e :deploy/status ?status]])
```

## Audit pattern — what changed, when

Bi-temporal reads make history an ordinary query, not a log replay. To see a
fact's life, query it with `:valid-at :any-valid-time` at successive
`:as-of` counters and diff:

```datalog
(query [:find ?s :as-of 2 :valid-at :any-valid-time :where [?e :deploy/status ?s]])
```

```datalog
(query [:find ?s :as-of 5 :valid-at :any-valid-time :where [?e :deploy/status ?s]])
```

## Error codes you will actually see

| Code | Meaning | Do |
|---|---|---|
| `PRS-001` | Datalog failed to parse | Check parens, leading `(`/`[` shapes, quoted strings |
| `PRS-078` | Malformed query shape (e.g. clauses after `:where`) | Clauses go *inside* the `[:find ... :where ...]` vector |
| `STG-026` | Another process holds the graph file | Use the jinn instance that owns it; don't fight the lock |
| `STG-025` | Already open in this process | Internal guard; report it if you see it |
| size errors | Fact over ~4 KB | Store a path/URL, not the payload |

Tool errors render as `[CODE] message`. `transact`/`query` on an unknown
graph refuse with "create it with create_graph first".

## Multi-valued attributes — the #371 workaround

One value per transact (the server tool description repeats this):

```datalog
(transact [[:order/9 :order/tag "priority"]])
```

```datalog
(transact [[:order/9 :order/tag "gift"]])
```

Reading both values back needs no special shape — the query matches a value
per row.
