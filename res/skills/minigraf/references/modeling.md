# Modeling durable state in minigraf

Conventions that keep a shared graph legible across sessions, subagents, and
restarts. Generic graph shapes only — the workflow that uses them is yours to
design.

## Entity identity

Keyword entities are deterministic: `:task/12` is the same entity in every
session, forever. Derive names from something stable (an id, a slug, a path):

```datalog
(transact [[:file/src-main-rs :file/lines 213]
           [:file/src-main-rs :file/owner :team/core]])
```

`#uuid "..."` literals are accepted when the identity comes from outside
(session ids, request ids). Don't invent UUIDs to encode hierarchy — a
`:parent` edge is simpler than a composite id.

## Naming conventions

- Attributes: `:domain/field` — `:task/status`, `:build/started-at`,
  `:edge/to`. A reader can group the graph by prefix.
- Status-ish values: `:verb/word` keywords — `:open`, `:status/done`,
  `:build/failed`. Keywords sort and compare exactly.
- Timestamps: ISO-8601 strings — `"2026-10-08T07:00:00Z"`. They sort
  lexicographically, and `:valid-at` accepts the same shape.

## State machines

Model state as an attribute holding a keyword plus an edge per transition,
so both "where is it" and "how did it get here" are one-hop queries:

```datalog
(transact [[:job/render :job/state :running]
           [:job/render :job/state-when "2026-10-08T07:00:00Z"]])
```

```datalog
(retract [[:job/render :job/state :running]])
```

```datalog
(transact [[:job/render :job/state :done]])
```

```datalog
(query [:find ?state :where [?e :job/state ?state]])
```

Keep one current-state fact per entity (retract-then-write — see SKILL.md),
because it makes "everything in state X" a single pattern.

## DAGs and trees

Edges are facts; traversal is a recursive rule. Remember rules are
memory-only — register at the start of a session that needs them.

```datalog
(transact [[:step/a :edge/to :step/b]
           [:step/b :edge/to :step/c]])
```

```datalog
(rule [(reach ?x ?y) [?x :edge/to ?y]])
```

```datalog
(rule [(reach ?x ?y) [?x :edge/to ?m] (reach ?m ?y)])
```

```datalog
(query [:find ?to :where (reach :step/a ?to)])
```

Cycles are safe (the rule engine memoizes), but model what is actually a DAG
as one — add a `:depth` or `:rank` fact when order matters.

## Recording without blocking

The graph is shared state, not a message queue: transact the fact when the
decision is made, keep the fact minimal, and let the next reader query what
it needs. If a value would be more than a few hundred bytes, write it to a
file and store the path.

## Exporting everything

There is no export tool. The all-facts read is:

```datalog
(query [:find ?e ?a ?v :valid-at :any-valid-time :where [?e ?a ?v]])
```

Add `:as-of <counter>` for a point-in-time snapshot. This is the basis for
copying a graph's data (write it into a fresh graph with new `transact`
calls).
