---
name: minigraf
description: Write and query durable graph state through the jinn minigraf MCP server (mcp__minigraf__* tools) — bi-temporal facts, Datalog transacts/queries, time travel. Use when creating or updating graphs, recording workflow or task state that must survive restarts, or asking what is still pending/true/history in a shared graph.
---

# minigraf — the jinn minigraf server

You are writing durable, structured state to a shared bi-temporal graph through
the `mcp__minigraf__*` tools. Every session of this jinn process shares the same
graphs: what you write, other sessions and subagents can read.

## The four tools

| Tool | Does |
|---|---|
| `mcp__minigraf__create_graph(name)` | Creates `<name>.graph`. **Fails if it exists** — the existing graph is untouched. Names: `a-z 0-9 _ -` only. |
| `mcp__minigraf__transact(graph, datalog)` | One write command: `(transact ...)`, `(retract ...)`, or `(rule ...)`. Returns the tx id. |
| `mcp__minigraf__query(graph, datalog)` | One `(query ...)`, including time travel. Rows come back as JSON. |
| `mcp__minigraf__list_graphs()` | Graph names that exist. |

`create_graph` before first use of a name; `transact`/`query` on an unknown
name are refused with an error telling you to create it. There is no delete
tool — graphs persist forever by design.

## The three rules that keep state correct

**1. One value per (entity, attribute) per transact.** Two values for the same
attribute written in one call can read back as one (minigraf #371, fixed in
v3). Multi-valued attributes take one `transact` call per value:

```
(transact [[:order/42 :order/tag "urgent"]])     ← call 1
(transact [[:order/42 :order/tag "gift"]])       ← call 2, same entity+attr, different call
```

**2. Facts are small — pointers, not payloads.** A fact must stay well under
4 KB. Store a file path or URL in the fact, not the file contents.

**3. Create before use, never overwrite.** Only `create_graph` makes a graph,
and it never replaces one. Nothing else you can do corrupts existing state.

## Minimal cycle

```datalog
(transact [[:task/1 :task/title "ship minigraf server"]
           [:task/1 :task/status :open]])
```

```datalog
(query [:find ?title
        :where [?e :task/status :open]
               [?e :task/title ?title]])
```

Facts are `[entity attribute value]` triples; queries are conjunctive
`[?var :attr ?val]` patterns. Entities written as `:task/1` are keywords —
stable identity without minting UUIDs. Attributes are namespaced
`:domain/field`; values are strings, numbers, booleans, or `:keywords`.

## Correcting state — retract, don't overwrite

History is the point: retract the wrong value and transact the new one. Both
remain queryable in the past (see `references/datalog.md` for time travel).

```datalog
(retract [[:task/1 :task/status :open]])
```

```datalog
(transact [[:task/1 :task/status :done]])
```

## After a server restart: rules are gone

`(rule ...)` definitions live in memory only — a jinn restart (or
`restart_mcp_server`) forgets them, and the facts are all still there.
Re-register any rule your queries need before querying. Prefer writing the
facts a question needs over leaning on rules.

## Where to look next

| File | Covers |
|---|---|
| `references/datalog.md` | Full command grammar, `:as-of`/`:valid-at` time travel, audit patterns, aggregate/`not` clauses, error codes |
| `references/modeling.md` | Entity identity conventions, state machines, DAGs, naming, the all-facts export idiom |
