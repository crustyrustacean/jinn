+++
name = "gap-analysis"
description = "Check for gaps in the implementation versus the acceptance criteria"
+++

<instructions>
Identify any gaps in the implementation versus the acceptance criteria and the original approved plan and discussion. Generate a table consisting of:
- All acceptance criteria, identified as "AC#" (AC1, AC2, ...)
- Whether the criteria was met
- _What_ confirms that the criteria was met or not
- Expected features from the plan/discussion
- Whether those features were implemented

For all gaps identified:

- Include the acceptance criteria/feature identifier from the table
- Explain _why_ the gap exists
- Propose a fix

## Record reconciliation

Read `.agents/RECORD.md` if it exists. Check whether the implementation changed a behavior described by any recorded entry — this work is authoritative about the current state, and a change on the ground that diverges from a recorded fact is a drift signal that must be surfaced, not silently left.

- For each recorded entry the implementation changed, broke, or made stale, flag it as a gap and propose the exact amended (or removed) entry verbatim.
- For each "Record Updates" entry the approved plan promised to apply, check that it was actually written into `.agents/RECORD.md`. If it was not, flag the omission as a gap.
- If the work established a new high-level fact that has no entry yet, propose a verbatim entry following the record's format rules.
- Do not flag cosmetic or unrelated edits; only surface entries whose stated behavior diverged or was newly established.
  </instructions>
