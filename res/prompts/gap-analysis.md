+++
name = "gap-analysis"
description = "Check for gaps in the implementation versus the acceptance criteria"
+++

<instructions>
Identify any gaps in the implementation versus the approved spec/plan and discussion. Generate a table consisting of:
- All acceptance criteria, identified as "AC#" (AC1, AC2, ...)
- Whether the criteria was met
- _What_ confirms that the criteria was met or not
- Expected features from the plan/discussion
- Whether those features were implemented

For all gaps identified:

- Include the acceptance criteria/feature identifier from the table
- BRIEFLY Explain _why_ the gap exists
- Propose a fix

## Record reconciliation

If the spec/plan included "Record Updates", then you should also **confirm accuracy against the actual implementation**. Read `.agents/RECORD.md` if it exists. The implementer writes Record updates at the end of implementation (via the "Update the Record" task), so by the time you check, the Record should already reflect this work.

- For each "Record Updates" entry the approved plan promised, confirm it was written into `.agents/RECORD.md` and that it **matches what was actually implemented**. If it was not written, or if what was written does not match the implementation, flag the omission/mismatch as a gap.
- If the implementer surfaced a **divergence** (implementation did not match the planned entries, so it wrote nothing), verify that divergence is genuine — then either propose a correct verbatim entry for the user to approve.
- For any recorded entry the implementation changed, broke, or made stale that was **not** covered by the planned Record Updates, flag it as a gap and propose the exact amended (or removed) entry verbatim.
- If the work established a new high-level fact that has no entry yet, propose a verbatim entry following the record's format rules.
- Do not flag cosmetic or unrelated edits; only surface entries whose stated behavior diverged or was newly established.
  </instructions>
