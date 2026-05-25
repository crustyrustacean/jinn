+++
name = "approve-plan"
description = "Generate a context-rich implementation specification from an approved plan."
+++

<instructions>

The user has approved the High-Level Pitch discussed previously. Your task is to write the **Context-Rich Specification** to disk.

## Instructions

1.  **Synthesize History:** Review the entire Socratic dialogue and the approved High-Level Pitch. Extract all technical decisions, code references, and algorithms discussed.
2.  **Generate Specification:** Write a comprehensive, standalone technical specification to `.plans/<task>/plan.md` (where `<task>` is a slugified version of the task name).
3.  **Report:** Reply to the user with "Plan created at .plans/<task>/plan.md".

## Mandatory Elements

The specification must be a **standalone technical document**. A fresh agent with no prior memory must be able to implement the feature using only this document. It MUST include the following sections explicitly:

1.  **Dialectical Outcomes (Why):** Reasoning for key decisions based on the Socratic dialogue. Document trade-offs and alternatives rejected.
2.  **Relevant Files (Where):** A list of specific files to be created or modified, with full paths.
3.  **Key Code Context (What):** Snippets of existing code that the implementation depends on or must modify (e.g., struct definitions, function signatures). Do not just reference them; include the code blocks.
4.  **Implementation Algorithm (How):** The explicit logic for implementation. Detail state machines, logic flows, or data transformations.
5.  **Anti-Goals (Out of Scope):** Explicitly list what is _not_ being implemented to prevent scope creep.
6.  **Edge Cases & Gotchas:** Highlight technical pitfalls or tricky logic discovered during the dialogue.
7.  **Navigation Anchors:** Identify specific functions or modules that serve as primary entry points for the changes.
8.  **Dependency Mappings:** List new external libraries or internal module dependencies required.
9.  **Test Strategies:** Specific guidance on _how_ to verify each phase (e.g., "Update unit test X", "Add test for edge case Y").

## Structure

- The document **must** begin with the "Problem" and "Solution" from the Pitch.
- The document **must** include the "Acceptance Criteria" from the Pitch.
- The document **must** include the "Phases" (with checkboxes) from the Pitch, but expanded with implementation specifics.
- The document **must** have dedicated headers for all Mandatory Elements listed above.

</instructions>
