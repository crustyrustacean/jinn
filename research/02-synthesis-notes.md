# Mixed-Concurrency Turn Composition — Research Notes

Compiled 2026-10-01. Working notes; the deliverable is the chat summary.

## The load-bearing reframing

The user's original framing was "batch tool calls OR write a script." They rejected it and
correctly so: a turn should contain BOTH several discrete concurrent tool calls AND a
self-written fan-out script. The hard problem is therefore not parallelism but the
**partition decision** — which work becomes a script, which stays a discrete call.

Findings below are organized to support that partition decision.

## Source inventory

| Bucket | Status | Weight |
|---|---|---|
| Primary artifacts (extracted/OSS prompts) | 8 agents | primary |
| Academic / preprint | ~12 papers | primary |
| Official vendor docs | Anthropic, OpenAI, OpenHands, vLLM | primary for mechanics |
| Issue trackers | ~15 issues | supporting |
| Practitioner blogs / forum | few | supporting |

## 1. Harness mechanics (prerequisite, not a prompt problem)

### Anthropic (official docs, verified)

- Wire format: multiple `tool_use` blocks in one assistant turn, `stop_reason: tool_use`.
- **The API does not execute anything.** Docs: "How you run those calls is your decision.
  The API doesn't prescribe an execution order."
- Parallelism is ON by default. Off via `disable_parallel_tool_use: true` *inside*
  `tool_choice` — explicitly "not a top-level request parameter."
  - `tool_choice.type: auto` + disable → at most one call per response
  - `tool_choice.type: any|tool` + disable → exactly one call
- Result formatting is load-bearing: all `tool_result` blocks must go in ONE user message,
  no text before them. Wrong formatting "teaches" Claude to avoid parallel calls.
- Every `tool_use` needs a matching `tool_result`, including skipped calls (`is_error: true`).
- Official measurement metric: `avg_tools_per_message`, "should be > 1.0."

Official system prompt (verbatim, recommended for Claude 4+):

```
<use_parallel_tool_calls>
For maximum efficiency, whenever you perform multiple independent operations, invoke all
relevant tools simultaneously rather than sequentially. Prioritize calling tools in
parallel whenever possible. For example, when reading 3 files, run 3 tool calls in
parallel to read all 3 files into context at the same time. When running multiple
read-only commands like `ls` or `list_dir`, always run all of the commands in parallel.
Err on the side of maximizing parallel tool calls rather than running too many tools
sequentially.
</use_parallel_tool_calls>
```

Second official clause, for the dependency guard:

> "Only batch tool calls that are independent of each other."

**Model caveat, official and important:** Claude Fable 5.1 "may issue fewer parallel tool
calls than earlier models, most noticeably in long agent loops where the next reads are only
implied (custom coding agents, bash and text editor harnesses, computer use). Standard
function calling is unaffected."

### OpenAI (official API reference, verified)

- `parallel_tool_calls` on Chat Completions: "Whether to enable parallel function calling
  during tool use." Optional bool.
- Responses API: same field, `"default": true` per the OpenAPI schema.
- Agents SDK doc claimed default False; the Python client actually defaults True. Docs/impl
  mismatch, fixed in openai-agents-python#762 (2025-05-26, closed).
- vLLM: setting `parallel_tool_calls: false` "ensures vLLM only returns zero or one tool
  call per request"; default true, but "There is no guarantee more than one tool call will
  be returned ... that behavior is model dependent and not all models are designed to
  support parallel tool calls."

### Harnesses that default to SERIAL — the biggest practical gotcha

- **Codex CLI**: MCP tool calls run serially by default; per-server opt-in
  `supports_parallel_tool_calls = true` (reported since v0.121.0). Source is a third-party
  blog, NEEDS-CHECK against the repo.
- **OpenHands SDK**: `tool_concurrency_limit`, default 1, "still experimental." Verified in
  official docs example code (`tool_concurrency_limit=4` / `=8` set explicitly).

## 2. Prompt text that actually ships (primary artifacts)

### Claude Code (leaked extractions, 4 independent sources agree)

> You have the capability to call multiple tools in a single response. When multiple
> independent pieces of information are requested, batch your tool calls together for optimal
> performance. When making multiple bash tool calls, you MUST send a single message with
> multiple tools calls to run the calls in parallel. For example, if you need to run
> "git status" and "git diff", send a single message with two tool calls to run the calls in
> parallel.

### Cline (OSS source) — the closest existing artifact to the user's ask

> You can call multiple tools in a single response. Before using tools, identify every
> independent read, search, command, or edit needed for the next step and emit all of those
> tool calls now, either as multiple tool calls or as one batched input for tools that
> accept arrays. Do not wait for one independent result before requesting another. Do not
> split independent reads, searches, checks, or edits across separate turns.

Cline's few-shot block is the best parallelism exemplar list found in any prompt:

> Good parallelism examples: read all known relevant files in one read_files call; run
> independent inspection commands in one run_commands call; emit independent read_files,
> search_codebase, and run_commands calls together in one response; emit multiple editor calls
> together when editing different files or non-overlapping regions.

### Claude Code's script fan-out (Workflow tool) — the "shape (b)" precedent

The only real script-fan-out primitive in any corpus. `parallel(thunks)` / `pipeline(...)`,
with an explicit barrier semantic documented. Gated hard:

> For any other task — even one that would clearly benefit from parallelism — do NOT call
> this tool.

Conflict-safety clause (unique to Claude Code): fan-out children are told they are not alone
in the environment so they don't clobber each other.

### Pushing AWAY from scripts

Claude Code: "While the Bash tool can do similar things, it's better to use the built-in tools
as they provide a better user experience and make it easier to review tool calls and give
permission." Stated reason is permission-review ergonomics, not efficiency.

### Negative findings

- **Aider**: no batching prompt. **Goose**: no batching prompt; its subagent prompt is
  actively anti-batching ("Use the minimum number of tools needed").
- **No agent in the corpus has a parallel example using an MCP tool.** Every concrete example
  is a shell read.
- The most-replicated sentence across 4 independent authors is the negative example:
  "if one operation must complete before another starts, run these operations sequentially
  instead."
- Version drift: older Claude Code gist (mid-2025) text is louder but obsolete — the clause
  was deduplicated from tool descriptions and a restraint section added later.

## 3. Academic evidence

### The paper the user half-remembered EXISTS

**"When Does Restricting a Coding Agent to `execute_code` Help?"** (arXiv 2607.10569,
Yang/Yu/Desell, Rochester, Jul 2026, non-archival KDD workshop paper). Three arms:
baseline / bash_only / **code_only (single `execute_code` tool as only action)**.

**It contradicts the thesis the user expected.** Pass rates statistically tied in every
(regime × agent) cell; differences <3 points while costs swing 20–40%. Authors: "It does not
support model-capability claims." The effect is cache-adjusted cost only.

Useful sub-finding: tool calls per LLM call **1.35 (baseline) vs 0.89 (code-only)** — the
only direct measurement of batching-as-behavior found anywhere. And: "Claude's API
empirically does not batch on SWE-bench (30.1 vs 27.9 calls/run)" — opposite sign from Codex.

### CodeAct (ICML 2024, peer-reviewed) — and why it does NOT support "batching"

The factorial decomposition is the valuable part:
- Atomic single-tool calls (§2.2): gain **absent**; open-source models often lose to plain text
- Compositional tasks (§2.3): gain **present**, 12/17 models
→ The mechanism is **compositionality** (control flow, local variables), not parallelism.

Also: CodeAct's own system prompt tells the agent to "attempt fewer things at a time instead
of putting too much code in one execute block" — the flagship code-as-action agent is nudged
AGAINST batching.

### W&D (arXiv 2602.07359, Stanford, verified) — the only study varying parallelism

BrowseComp: 1 tool → 66% acc / $102.50 / 1522.6s; 3 tools → 68% / $65.70 / 904.2s
(**−35.9% cost, −40.6% wall-clock**).

**But the comparison confounds batching with model behavior** — higher accuracy AND lower
cost. Not an equal-accuracy ablation.

The critical finding for prompt design:
> The Automatic strategy did not perform better than Descending, indicating that the LLM
> itself cannot determine the optimal number of tool calls in each iteration.

Schedulers: **Descending (74) > Automatic (72) > Constant-3 (68) > Constant-1 (66) > Ascending (63)**.
→ A static "batch more" clause asks the model to do the one thing measured as beyond it.

Also: **per-turn user message beat system-message instruction** for enforcing a call count.
And: open-source models barely benefit.

### "The Bitter Lesson of Tool Calling" (arXiv 2608.06370, PwC preprint)

- Chaining: PTC advantage **18.8% absolute at chain ≥12**, absent at short chains.
  Mechanism: JSON costs one extra inference turn per link.
- Fan-out: Claude Sonnet 5 enumeration **100% at N≤70, 75% at N=72, 0% at N=100** —
  catastrophic, not graceful. GPT-5 degrades above ~13 in the same experiment.
- **But** its own per-category table shows PTC **−14.1% on BFCL parallel categories**,
  attributed to three models emitting literal `\n` instead of newlines → SyntaxError.
- **Scoring caveat**: "models frequently produce the correct aggregation answer from
  parametric world knowledge ... without executing the enumeration calls." Success rate
  over-counts fake fan-out. Measure enumeration, not output.
- Small n, wide CIs, no multiple-testing correction, PwC-authored.

### Fan-out ceilings disagree — and the harness ceiling is much lower

| Number | What it is |
|---|---|
| ~13 | GPT-5 practical enumeration degradation |
| 70–72 | Claude Sonnet 5 onset (0% at 100) |
| ≥4 | Hermes Agent: ALL results lost (a harness bug) |
| 3 | Claude Code: 2 of 3 parallel results lost |

Harness ceilings are ~an order of magnitude below model ceilings.

## 4. Failure modes

- **Serialization**: Codex MCP serial-by-default; OpenHands `tool_concurrency_limit=1`.
- **Dependency races**: `git add` → `git commit` raced in Codex (#13963). Model error was
  "not temporally aware that commands take time to execute." Reporter's design principle:
  "Sequential operations should have time between the *completion* of a step and the start of
  the next step."
- **Larger models batch MORE wrongly**: ToolSandbox (2408.04682) — "larger models like GPT-4
  and Claude-3-Opus perform significantly worse ... due to erroneous parallel tool calls in
  face of state dependency." Inverts the usual assumption. NEEDS-CHECK.
- **Silent partial loss**: Claude Code #63859 — "one failing call cancels every other call in
  the same batch."
- **Script blast radius**: GPT-4.1 98.1% → 40.4% on chaining from a single `\n` encoding
  bug. One formatting error zeroes the whole turn; no partial credit.
- **Over-batching changes the plan**: 429s from 8 concurrent calls led a model to read six
  failures as "search is broken" and abandon a correct strategy. (Source weak; mechanism sound.)
- **Cursor at scale**: 20 agents → throughput of 2–3. "The harness and models matter, but the
  prompts matter more."
- **Prompt cache TTL**: 5 min. Batching helps here — collapsing wall-clock keeps the cache
  hot. Nobody in the fan-out debate mentions this.

## 5. Evaluation hazards

- Jesse Vincent built MCP file tools to save tokens; cost **+34%**. The real 7× win was plain
  `sed -n` ranged reads. Method note: "Codex runs swing ±40k tokens; a clean +12% signal hides
  in that." → End-to-end A/B may be unable to detect a modest batching win.
- Composite tool-use scores (e.g. CTUR) bundle abstention with correctness — not batching metrics.
- Fake numbers circulating: "3.7x latency cut" traces to a page that self-declares its figures
  illustrative. Discard.

## 6. The open ground (most important section)

1. **No A/B test of a "batch independent tool calls" system-prompt clause exists.**
2. **No controlled study of few-shot exemplars teaching batching.** The only lever studied is
   a direct instruction. This is net-new territory.
3. **No source anywhere tests concurrent tool calls + a fan-out script in the same turn.**
   Bitter Lesson tests PTC *instead of* JSON parallel, never alongside.
4. **No paper measures batching rate as an outcome metric in itself.**
5. **No factorial design separates composition / self-correction / parallelism.**
6. **Almost no coding-agent evidence** — the two strongest papers are deep-research and
   function-calling benchmarks with echo-stubs.
7. No postmortem of a fan-out script destroying prior work; no measured cost of over-batching.
8. Prompts cannot reliably enforce caps under pressure (Hermes reporter's explicit finding).