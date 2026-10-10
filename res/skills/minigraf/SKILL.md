---
name: minigraf
description: Write and query durable graph state through the jinn minigraf MCP server (mcp__minigraf__* tools) — bi-temporal facts, Datalog transacts/queries, time travel, and reusable graph templates with replay-on-open rules. Use when creating or updating graphs, recording workflow or task state that must survive restarts, or asking what is still pending/true/history in a shared graph.
---

# minigraf — the jinn minigraf server

You are writing durable, structured state to a shared bi-temporal graph through
the `mcp__minigraf__*` tools. Every session of this jinn process shares the same
graphs: what you write, other sessions and subagents can read.

## Your active graph

You always have at most **one active graph**. Calls to `transact` and `query`
take no graph argument — they hit your active graph. jinn binds it for you:
instantiate, create, or attach once, then just transact and query. If a call
comes back "no graph active", bind one first.

## The six tools

| Tool | Does |
|---|---|
| `mcp__minigraf__list_templates()` | Templates available for instantiation. |
| `mcp__minigraf__instantiate_graph(template, name)` | Creates `<name>.graph` from a template's seed script and binds it as your active graph. **Fails if the name exists** — the existing file is untouched. |
| `mcp__minigraf__create_graph(name)` | Same, but a blank graph (no template). |
| `mcp__minigraf__attach_graph(name)` | Binds an **existing** graph as your active graph. |
| `mcp__minigraf__transact(datalog)` | One write command against your active graph: `(transact ...)`, `(retract ...)`, or `(rule ...)`. Returns the tx id. |
| `mcp__minigraf__query(datalog)` | One `(query ...)` against your active graph, including time travel. Rows come back as JSON. |

Names: `a-z 0-9 _ -` only. There is no delete tool — graphs persist forever by
design.

## The three rules that keep state correct

**1. One value per (entity, attribute) per transact.** Two values for the same
attribute written in one call can read back as one (minigraf #371). This bites
template authors constantly — see the template section below. Multi-valued
attributes take one `transact` call per value:

```
(transact [[:order/42 :order/tag "urgent"]])     ← call 1
(transact [[:order/42 :order/tag "gift"]])       ← call 2, same entity+attr, different call
```

**2. Facts are small — pointers, not payloads.** A fact must stay well under
4 KB. Store a file path or URL in the fact, not the file contents.

**3. Create before use, never overwrite.** Only `instantiate_graph` and
`create_graph` make graphs, and neither replaces one. Nothing else you can do
corrupts existing state.

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

## Templates — repeatable workflows

A template is a directory under the jinn config dir's `graphs/` folder holding
a hand-editable `template.dl` script. `list_templates` lists what exists;
`instantiate_graph` stamps out an instance per workflow run (or per session)
so every run starts from the same shape.

A template script has two kinds of forms:

- **Seed forms** — `(transact ...)`/`(retract ...)`. Run **once**, when the
  graph is instantiated. They never run again: re-running them would overwrite
  timestamped facts and re-fire retracts.
- **Rule forms** — wrapped in one top-level `(rules [...])`. Registered at
  instantiate time **and re-applied automatically every time the graph is
  opened fresh** (server restart, jinn restart). Facts live in the file; rules
  live in memory — the replay is what makes a template's derived relations
  survive restarts.

```datalog
;; seed: runs once at instantiate_graph
(transact [[:step/1 :step/status :ready]])
(rules [
  (rule [(ready? ?s) [?s :step/status :ready]])
])
```

You can author a template yourself: write the directory and `template.dl` with
the `write` tool (each top-level form must be a `(transact ...)`,
`(retract ...)`, or the single `(rules [...])` wrapper), then instantiate from
it. If a template directory carries sibling files — commonly
`steps/01-do-something.md` — they belong to the workflow: a step entity can
point at its instructions with a fact like
`[:step/1 :step/instructions "/path/to/template/steps/01-do-something.md"]`,
and whoever executes the step reads that file. The server ignores everything
but `template.dl`.

**After a restart**: an instantiated graph's rules come back on their own (the
server replays the template). Rules you registered by hand with a bare
`(rule ...)` transact do **not** — they are gone; re-register them or move them
into a template.

## Where to look next

| File | Covers |
|---|---|
| `references/datalog.md` | Full command grammar, `(rules [...])` shape, `:as-of`/`:valid-at` time travel, audit patterns, aggregate/`not` clauses, error codes |
| `references/modeling.md` | Entity identity conventions, state machines, DAGs, naming, template design, the all-facts export idiom |
