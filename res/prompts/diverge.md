+++
name = "diverge"
description = "Find approaches nobody suggested yet"
+++

<instructions>

Come up with approaches nobody has suggested yet.

DO NOT SUGGEST APPROACHES YOURSELF IN THIS TURN.
DO NOT SHOW ANY SUBAGENT THIS CONVERSATION, YOUR REASONING, YOUR CONCLUSION, OR ANOTHER SUBAGENT'S APPROACH.

## This is a read-and-report turn

- **DO NOT CREATE, MODIFY, OR DELETE ANY FILE IN THIS TURN.**
- **DO NOT BEGIN THE NEXT UNIT OF WORK IN THIS TURN.** Generate and report; do not act.
- **Lead with the answer.** First line: what the approaches disagree about, in one sentence.
- **Keep the report under roughly 400 words** unless the user asked for more detail.
- **End with a state-of-the-work line**, past tense: what work was in flight, and whether anything
  above changes it. Phrase it as a fact, never as a next step.

## Step 1: See what's already ruled out

List the approaches that have already come up. They divide the space up: rewrite versus patch,
in-process versus external, bring in a dependency versus write it yourself. Those are the edges of
the space. What they all agree on is what nobody's questioned yet — and that's where the interesting
ones live.

Say what the existing options rule out, then proceed. Do not stall on a round trip.

## Step 2: Give each subagent a different constraint

**2 subagents**, spawned in parallel with the `task` tool. Each gets a different constraint. Say how
many you're spawning before spawning them.

The two constraints have to push in genuinely different directions. Two that both say "think
outside the box" are the same constraint, and you'll get two of the same idea.

Things that actually move the thinking:

- **Go outside the usual categories.** The approaches already suggested all sit inside conventional
  ways of doing this. Say what category they're in, then go somewhere else.
- **Make the obvious answer wrong.** Solve it such that the normal approach is the thing that fails.
- **Break a constraint on purpose.** Answer with the hardest requirement ignored, then say what
  ignoring it costs and whether that cost is worth paying. Satisfying constraints too early cuts off
  directions before they've been looked at.
- **Wild or unexpected framing.** "Wild", "unconventional", "what's the opposite of what's expected".
  Negative framing reaches further than positive framing like "respectful" or "collaborative".
- **A different layer.** Solve it somewhere else entirely — a different part of the system, a
  different point in the lifecycle, a different moment.
- **One option that would work in a different system.** Bring it over and see if it holds.

Don't assign a named creativity method like SCAMPER or TRIZ. Measured against plain instructions,
those don't make ideas more novel — saying "think outside the box" works better and takes less room.

## Step 3: Brief the subagents

One `task` call per constraint — two calls. `description` names the constraint:
`diverge-categories` and `diverge-negative`, or whatever pair you picked.

```
<question>
What's being worked out, in full.
</question>

<already suggested>
The approaches that have come up. Don't repeat any of them, and don't propose a variation on one. If
you reach for one of these, you've gone back inside the space — get to the question a different way.
</already suggested>

<constraint>
<the specific constraint for this subagent>
</constraint>

<task>
Come up with one approach and work it through far enough that you can see what it costs: what it
actually means here, what it makes hard, what it gives up, what happens if you're wrong about it.
Pick one and commit. Don't list options, don't survey the space, don't finish with "one option
would be to…".
</task>
```

Only `<constraint>` changes between the two subagents. `<question>`, `<already suggested>`, and
`<task>` stay the same.

## Step 4: Report

```
## Approaches

| # | Constraint | In one line | Gets past | Gives up |
| --- | --- | --- | --- | --- |

## Where they disagree

<The things they actually disagree about — cost, how hard to undo, what they make easy later. The
thing they disagree on is the decision.>

## The surprising one

<The approach that went somewhere unexpected. Usually the only one carrying information the user
doesn't already have. If there isn't one, say so.>

## Went back inside

<Any approach that's really just one of the ones already suggested. Name it. If they all came back
inside, that's the finding: the options already cover this space, and the way forward is to question
one of the assumptions they share rather than to keep generating.>
```

- **Two approaches that work the same way are one approach.** Say they converged.
- **Coming back inside is a result too**, and a useful one — it tells the user their options were
  complete.
- **Report the counts** — subagents run, subagents failed.
- **Don't merge them.** Don't pick a favourite, don't write "a hybrid would take A's approach and
  B's framing." The approaches and what they disagree about are the answer.

## When this approach is the wrong one

- You want to know whether one thing is any good. Judge it instead.
- The question has a right answer you can get with a tool. Run the tool.

</instructions>

## SUBJECT