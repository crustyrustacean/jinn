+++
name = "research"
description = "Basic research on a topic"
+++

<instructions>
Your task is to perform research on a topic provided by the user at the end of this prompt. The user MUST provide:
- Research topic
- What it's for
- Perspective (slant/angle/bias/spin)

If the user fails to provide any of these, please ask the user to specify. Provide examples for them based on their initial prompt. So if they say something like "Marine life" you can ask "What's the purpose? (Research paper, report, etc)" and "What perspective? (Factual, extreme, biased towards <a|b>)" etc.

## Research brief

Once the required inputs are settled, run a few broad orientation searches on the topic, then present a **Research Brief** in chat: what you will search for, the biases and purpose driving it, and what the report will contain. The user approves, edits, or vetoes it before any real research starts. The brief states:

- **Objective**: purpose + angle, one line
- **Source Categories**: material scope and source tiers that count (if scope is ambiguous — e.g. "his writings" might mean just essays, or everything he's said/done — ask here; default broader, not narrower).
- **Query angles**: the kinds of searches you intend to run
- **Expected themes**: 5-8, one line each
- **Done when**: what the report must contain to be complete

DO NOT START THE FULL RESEARCH UNTIL THE USER APPROVES THE BRIEF. If there are outstanding questions, resolve them first. Revise on request, then proceed autonomously to completion. The brief is a compass, not a cage: chase leads outside it as they appear and note divergences at the end of the report.

Note that you are NOT to create the output artifact the user needs the research for. You are to provide the DATA for the user, assuming they will use it for their output artifact. Use their specific artifact as a guide for research provenance. For example, if it's for a research paper then you'd want to search other papers and reputable sources. If it's for a blog post on a tabloid then you want rumors and spicy commentary etc.

Keep anything relevant-adjacent you stumble past that doesn't fit the stated angle but fits the vibe (feuds, absurd incidents, surprising findings — depends on the task). List these at the end of the doc under "Rabbit holes": link + one line each.

## Citation format

All sources must be cited for provenance. It's not important whether the source is "reliable" or "trustworthy". That is the user's decision to make.

- Use markdown footnote references: cite inline as `[^1]`, and define the footnote at the bottom of the doc as `[^1]: [Title — Outlet, date](https://example.com/source)`.
- Footnotes render as clickable superscripts that jump to the source entry — don't fake it with plain `[1]` text or inline links.
- Number footnotes in order of first appearance. Every source gets exactly one footnote, reused everywhere it's cited.
- Never cite a source you didn't actually fetch or read. If a claim rests on a search snippet only, mark it NEEDS-CHECK instead of citing.

## Output location

Save all research output to `./research/` (create it if needed). Do not use `.plans/` or any other directory — this is research material, not an implementation plan.

</instructions>

## TOPIC
