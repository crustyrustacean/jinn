# The Record

A curated list of factual, scoped statements asserting the application's
**current** state. Authoritative for the present, never the future.

The planner consults this file before proposing a plan. If a feature
**contradicts** an entry here, the contradiction is surfaced before the plan
proceeds. If a feature **establishes a new high-level fact**, a verbatim entry
is proposed for human approval as part of the plan.

## Format Rules

- **Factual.** Assert how things are *now*. Never future intent ("we will...",
  "should..."). Each entry is the current state of the application.
- **Scoped.** Name what each entry applies to — repo, app, frontend, or a named
  subsystem. An unscoped fact (e.g. "uses Fossil") is ambiguous: is that the
  repo, or the app's supported VCS list? Always disambiguate.
- **High-level.** One-liners (a few sentences at most). Capture decisions and
  facts a planner needs, not implementation minutiae.

## Templates

| Pattern | Form | Example |
| --- | --- | --- |
| State | `[Scope] currently [does X / is Y].` | "The TUI's first screen at startup is the chat screen." |
| Persistence | `[Scope] persists [what] to [where].` | "Sessions persist to SQLite." |
| Flow | `[Input/event] is handled by [actor/subsystem], which [action].` | "File edits route through the `edit` tool, which validates `LINE#HASH` anchors." |
| Boundary | `[Scope] is bounded by [constraint].` | "Project discovery walks ancestors until a VCS root or `$HOME`, whichever comes first." |

## Absence

A missing record, or an un-recorded area, simply means the list has no entry
there yet. Absence is not a constraint — it is an open question, and a feature
that fills a gap may establish the first entry for that area (proposed for
human approval as part of the plan).

## Editing

Entries are added or amended only with human approval, via a "Record Updates"
section surfaced in the approved plan. The gap-analysis step checks whether
implementation actually changed a recorded fact and flags any needed update.

---

<!-- Add entries below. Keep them scoped, factual, and high-level. -->
