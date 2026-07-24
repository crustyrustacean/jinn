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

The implementation loop writes Record updates at the end of implementation (via the "Update the Record" task), so by the time you run, the Record should already reflect this work. Your job is to **confirm accuracy against the actual implementation**, not to write the record yourself. Read `.agents/RECORD.md` if it exists.

- For each "Record Updates" entry the approved plan promised, confirm it was written into `.agents/RECORD.md` and that it **matches what was actually implemented**. If it was not written, or if what was written does not match the implementation, flag the omission/mismatch as a gap.
- If the loop surfaced a **divergence** (implementation did not match the planned entries, so it wrote nothing), verify that divergence is genuine — then either propose the correct verbatim entry or leave it for the user.
- For any recorded entry the implementation changed, broke, or made stale that was **not** covered by the planned Record Updates, flag it as a gap and propose the exact amended (or removed) entry verbatim.
- If the work established a new high-level fact that has no entry yet, propose a verbatim entry following the record's format rules.
- Do not flag cosmetic or unrelated edits; only surface entries whose stated behavior diverged or was newly established.
  </instructions>
