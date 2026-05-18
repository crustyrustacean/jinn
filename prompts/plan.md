+++
name = "plan"
description = "Create a high-level phased implementation plan"
+++

Create a high-level implementation plan for the task detailed at the end of this prompt.

DO NOT IMPLEMENT THE PLAN. WAIT FOR USER APPROVAL.

## Instructions

1. Explore the problem space to understand the request.
2. Identify if there are existing patterns that the request fits in.
3. Break the work into numbered phases as needed.
4. Ask clarifying questions from the user to address ambiguities in the request or to make a decision on how to proceed.
5. When there are multiple viable paths to fulfill the requirements of the task, present options for the user to choose from.
6. Once the user answers all the questions, propose the plan as a regular response and WAIT FOR USER APPROVAL. DO NOT SAVE TO DISK.
7. _After_ the user accepts the plan, write a _context-rich plan_ (see next section) to `.plans/<task>/high-level.md` where `<task>` is a slugified version of the task name (derive this from the task info).
8. Report to the user "Plan created"

## Context-Rich Plans

A "context-rich" plan carries enough context for a fresh agent to derive the necessary information to create a detailed execution plan for any particular phase. It should include reasoning as to why certain decisions were made based on the conversation with the user while creating the high-level plan.

For example, if the user specifically questioned, or asked for a particular thing, or agreed to some specific thing, then those need to be mentioned in the relevant locations in the plan. This way the fresh agent understands why a particular path is being taken instead of a different path.

Any samples or examples that were discussed should also be part of the context-rich plan so that the agent knows exactly how something should happen.

## Notes

- All plans must use implementation phases (even 1 phase plans)
- Use markdown checkboxes (see below) for implementation phases so that we can check them off as we go.
- The plan **must** have an "Acceptance Criteria" section that shows the high-level goals.
- The plan **must** have a "Problem" and "Solution" section at the beginning so the user and agent both understand what's being solved and how.

### Coding task notes

**These only apply if your task is related to coding**:

- For coding tasks: All phases **MUST** _sync_ with trunk/main as the last step in _each phase_. This is to reduce merge conflicts and to prevent code from drifting away from established patterns.
- For fossil scm: `fossil merge trunk` will sync your branch with the latest trunk. NEVER MERGE YOUR BRANCH INTO TRUNK. Merging into trunk will be done manually and is outside the scope of planning.

### Checkbox example

- [ ] Phase 1: Foo the bar
  - [ ] subtask 1 brief description
  - [ ] subtask 2 brief description
  - [ ] _Sync branch with trunk and resolve merge conflicts_ <--- include on every phase (coding tasks only)
        ...

- [ ] Phase 2: Bar
  - [ ] subtask 1 brief description
  - [ ] _Sync branch with trunk and resolve merge conflicts_ <--- include on every phase (coding tasks only)
        ...

## TASK
