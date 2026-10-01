# Primary Artifacts: Tool-Call Batching Language in Production Coding-Agent Prompts

Scope: verbatim prompt text shipped by production coding agents re: tool-call batching / parallel tool use.
Compiled 2026-09-30. All quotes below were read from source files I fetched and opened; nothing is reconstructed from memory.

## Source-kind legend

| Label | Meaning |
|---|---|
| **(b) LEAK/EXTRACT** | Unofficial extraction of a proprietary vendor prompt. The vendor did not publish it. |
| **(c) OSS-SOURCE** | Prompt text read directly out of a public, permissively-licensed open-source repo. This is the vendor's real shipped text, just in the open. |
| **(a) VENDOR-DOC** | Official vendor documentation (not prompt text). |
| (d) paraphrase | My own words. Marked inline. |

---

## 1. Claude Code

Three independent extractions converge on nearly identical text, which raises confidence that the wording below is real and stable.

### 1a. The "Parallel tool call note" — the canonical clause

**Source:** `Piebald-AI/claude-code-system-prompts`, file `system-prompts/system-prompt-parallel-tool-call-note-part-of-tool-usage-policy.md`
**Kind:** (b) LEAK/EXTRACT · **Extraction version:** `ccVersion: "2.1.30"` per the file's own frontmatter; repo HEAD is v2.1.286 (2026-09-30).
**Where it sits:** a bullet inside the **`# Tool usage policy`** section of the main system prompt.

> You can call multiple tools in a single response. If you intend to call multiple tools and there are no dependencies between them, make all independent tool calls in parallel. Maximize use of parallel tool calls where possible to increase efficiency. However, if some tool calls depend on previous calls to inform dependent values, do NOT call these tools in parallel and instead call them sequentially. For instance, if one operation must complete before another starts, run these operations sequentially instead.

This is the single densest clause in the whole corpus: it bundles **capability grant** (sentence 1), **the dependency condition** (sentences 2-3), **the imperative** ("make all independent tool calls in parallel"), the **maximization demand** ("Maximize use of parallel tool calls where possible"), and the **stated motivation** ("to increase efficiency") into one paragraph. Item 3 of the brief (when NOT to batch) is covered by "no dependencies between them" + "do NOT call these tools in parallel" + the one-operation-must-complete example.

**Corroboration — a second extraction, different author, same text:**

**Source:** `alirezarezvani/cc-system-prompts`, `system-prompts/system-prompt-main-system-prompt.md` line 134 · **Kind:** (b) LEAK/EXTRACT. Same sentence, with one appended clause:

> - You can call multiple tools in a single response. If you intend to call multiple tools and there are no dependencies between them, make all independent tool calls in parallel. Maximize use of parallel tool calls where possible to increase efficiency. However, if some tool calls depend on previous calls to inform dependent values, do NOT call these tools in parallel and instead call them sequentially. For instance, if one operation must complete before another starts, run these operations sequentially instead. Never use placeholders or guess missing parameters in tool calls.

**Corroboration — a third extraction:**

**Source:** `kbandrews/claude-code-system-prompts`, `prompts/01_main_system_prompt.md` line 100 · **Kind:** (b) LEAK/EXTRACT. Byte-identical to the Piebald version, including the "Never use placeholders" sentence being absent.

**Corroboration — an OSS reimplementation that vendored the text:** OpenCode ships this same paragraph in `packages/opencode/src/session/prompt/anthropic.txt` line 83 (see §8). Four sources agreeing means the wording is genuine.

### 1b. The imperative variant (older, and the "MUST" phrasing the user asked about)

**Source:** gist `wong2/e0f34aac66caf890a332f7b6f9e2ba8f` ("Tools and system prompt of Claude Code")
**Kind:** (b) LEAK/EXTRACT · **Extraction date:** the embedded environment block says `Today's date: 2025/6/13`; model `claude-sonnet-4-20250514`. So this is a **mid-2025** snapshot — roughly 14 months older than the Piebald extraction.
**Where it sits:** second bullet of **`# Tool usage policy`**.

> - When doing file search, prefer to use the Task tool in order to reduce context usage.
> - You have the capability to call multiple tools in a single response. When multiple independent pieces of information are requested, batch your tool calls together for optimal performance. When making multiple bash tool calls, you MUST send a single message with multiple tools calls to run the calls in parallel. For example, if you need to run "git status" and "git diff", send a single message with two tool calls to run the calls in parallel.

**Emphatic?** High, and scoped narrowly. Note the shape: the capability grant is soft ("You have the capability to"), but the requirement is a hard **MUST** — and it is scoped specifically to *bash* tool calls, with a concrete `git status` / `git diff` example embedded.

The same gist, in the **Task (subagent) tool description**, attaches the batching demand to parallel agent launches:

> You have the capability to call multiple tools in a single response. When multiple independent pieces of information are requested, batch your tool calls together for optimal performance. ALWAYS run the following commands in parallel:
>    - Create new branch if needed
>    - Push to remote with -u flag if needed
>    - Create PR using gh pr create with the format below. ...

(The PR-creation sub-agent prompt; same gist, `agent-prompt-pull-request-creation` equivalent.)

**Same gist, Git-commit tool description** — a general preamble plus per-step parallel notes:

> You can call multiple tools in a single response. When multiple independent pieces of information are requested and all commands are likely to succeed, run multiple tool calls in parallel for optimal performance. The numbered steps below indicate which commands should be batched in parallel.

Note the extra hedge absent from the main system prompt: **"and all commands are likely to succeed."** This is a *confidence-gated* batching condition — a wrinkle the main clause does not have.

**Same gist, Grep / Glob / Read tool descriptions** — a softer, speculative framing that has since been *removed* (see the changelog note in §11):

> - You have the capability to call multiple tools in a single response. It is always better to speculatively perform multiple searches as a batch that are potentially useful.  *(Glob)*

> - You have the capability to call multiple tools in a single response. It is always better to speculatively read multiple files as a batch that are potentially useful.  *(Read)*

And in the **"Tone and style"** section, a few-shot example that demonstrates a parallel turn:

> user: write tests for new feature
> assistant: [uses grep and glob search tools to find where similar tests are defined, **uses concurrent read file tool use blocks in one tool call to read relevant files at the same time**, uses edit file tool to write new tests]

This is the closest thing in the corpus to a literal few-shot parallel example in the main prompt. Note how the model is told to *name the parallel mechanic inline in its thinking trace*.

### 1c. The compact one-liner in the harness prompt (current, 2.1.274)

**Source:** `Piebald-AI/...`, `system-prompt-system-prompt-harness-instructions.md` · **Kind:** (b), `ccVersion: "2.1.274"`
**Where:** the `# Harness` bullet list.

> - Prefer the dedicated file/search tools over shell commands when one fits. **Independent tool calls can run in parallel in one response.**

A single line doing two jobs at once — items 2 and 5 of the brief in one sentence.

### 1d. Subagents / backgrounding parallel work

**System-prompt level**, `system-prompt-tool-usage-subagent-guidance.md`, `ccVersion: "2.1.53"`:

> Use the ${AGENT_TOOL_NAME} tool with specialized agents when the task at hand matches the agent's description. Subagents are valuable for **parallelizing independent queries** or for **protecting the main context window from excessive results**, but they should not be used excessively when not needed. Importantly, avoid duplicating work that subagents are already doing - if you delegate research to a subagent, do not also perform the same searches yourself.

This gives the **context/token motivation** the brief asked about, explicitly: subagents parallelize *and* shield the main context window.

**Agent tool description**, `tool-description-agent-usage-notes.md`, `ccVersion: "2.1.280"` — the hard MUST for parallel agent launches, conditional on default steering mode:

> - If the user specifies that they want you to run agents "in parallel", you **MUST** send a single message with multiple ${AGENT_TOOL_NAME} tool use content blocks. For example, if you need to launch both a build-validator agent and a test-runner agent in parallel, send a single message with both tool calls.

**Backgrounding clause** (same file) — worth quoting because it tells the model *not* to poll, which is the flip side of backgrounding:

> - Agents run in the background by default. When an agent runs in the background, you will be automatically notified when it completes — **do NOT sleep, poll, or proactively check on its progress.** Continue with other work or respond to the user instead.
> - **Foreground vs background**: Pass `run_in_background: false` only when your very next action depends on the agent's result and nothing else could usefully happen while it runs — e.g., a research agent whose finding gates the edit you're about to make. Otherwise let it run in the background (the default) — this includes fire-and-forget work, independent investigations, and anything where the user might hand you something else in the meantime. **Wanting the result "next" is not enough on its own.**

### 1e. The counter-clauses: when NOT to fan out

Claude Code is the only agent in this corpus that ships an explicit **restraint** section. `system-prompt-subagent-delegation-restraint.md`, `ccVersion: "2.1.215"`:

> Subagents multiply cost and time: each one re-establishes context, re-explores, and reports back, and you then re-read its report. Delegate only when the payoff clearly exceeds that overhead. Before spawning, apply these tests:
> - Do the work inline when it is a small, bounded sub-task — a few file reads, one search, a short edit, a single check. Do not spawn a subagent for work you could finish yourself in a handful of tool calls.
> - **Do not fan out multiple subagents on a single small task. Parallel subagents are for genuinely independent, sizeable tracks (unrelated modules, a wide multi-file investigation), not for splitting one modest job into pieces.**
> - Do not spawn a subagent to review, re-verify, or double-check work you can verify inline. Verification that fits in your own loop belongs in your own loop.
> - If you delegate, commit to the delegation: do not redo the subagent's work while waiting, and do not re-derive its findings once it reports. If you find yourself repeating what a subagent is doing, you should not have spawned it.
> - Keep spawn counts low. One well-briefed subagent for a large independent chunk is worth more than several loosely-briefed ones; brief it precisely the first time rather than launching, waiting, and re-briefing.
>
> Delegate for work that is genuinely independent, large enough to justify a fresh context, or naturally parallel. Otherwise, do it yourself.

**Coordinator mode**, `system-prompt-coordinator-mode-orchestration.md` — the most explicitly balanced statement anywhere in the corpus, and the only one to name the *tradeoff* in both directions:

> **Parallelism is your superpower for work that splits into genuinely independent pieces. Workers are async. Launch independent workers concurrently — don't serialize work that can run simultaneously. When doing research, cover multiple angles. To launch workers in parallel, make multiple tool calls in a single message. But don't parallelize simple tasks: a question or small task that takes a handful of tool calls is faster done in a single loop (one worker) than fanned out.**
>
> Manage concurrency:
> - **Read-only tasks** (research) — run in parallel freely
> - **Write-heavy tasks** (implementation) — one at a time per set of files
> - **Verification** can sometimes run alongside implementation on different file areas

The read-only/write-heavy split is a **conflict-safety rule** no other agent states.

### 1f. The "self-written script that fans out" — the Workflow tool

This is the direct counterpart to the user's requirement (b). `tool-description-workflow.md`, `ccVersion: "2.1.248"`:

> Execute a workflow script that orchestrates multiple subagents deterministically. Workflows run in the background — this tool returns immediately with a task ID, and a <task-notification> arrives when the workflow completes. Use /workflows to watch live progress.
>
> **ONLY call this tool when the user has explicitly opted into multi-agent orchestration. Workflows can spawn dozens of agents and consume a large amount of tokens; the user must request that scale, not have it inferred.** Explicit opt-in means one of:
> - The user included the keyword "ultracode" in their prompt (you'll see a system-reminder confirming it).
> - Ultracode is on for the session (a system-reminder confirms it) — see **Ultracode** in the workflow authoring reference.
> - The user directly asked you to run a workflow or use multi-agent orchestration in their own words ("use a workflow", "run a workflow", "fan out agents", "orchestrate this with subagents"). The ask must be in the user's words — a task that would merely benefit from a workflow does not count.
> - The user invoked a skill or slash command whose instructions tell you to call Workflow.
> - The user asked you to run a specific named or saved workflow.
>
> **For any other task — even one that would clearly benefit from parallelism — do NOT call this tool.** Use the ${AGENT_TOOL_NAME} tool (if available) for individual subagents, or briefly describe what a multi-agent workflow could do and how much it would roughly cost, and ask the user whether to run it.
>
> Every script must begin with `export const meta = {...}`: a PURE LITERAL (no variables, calls or interpolation) giving the workflow's `name`, a one-line `description` (shown in the permission dialog) and optionally `phases` — one `{ title, detail? }` per phase() call, titles matched exactly. **Pass the script inline via `script` — do not Write it to a file first**, and do not also set the tool's `name` input (that selects a saved workflow); it is plain JavaScript, not TypeScript.
>
> The canonical multi-stage pattern — pipeline by default, each dimension verifies as soon as its review completes:
>   [full JS example using `pipeline(DIMENSIONS, ...)` with `parallel()` nested in the verify stage]

The embedded example is a real fan-out script, commented:

>   // Dimension 'bugs' findings verify while dimension 'perf' is still reviewing. No wasted wall-clock.

**Notable tension worth flagging to the user:** Claude Code ships a maximalist parallel clause *and* a restrictive opt-in gate on the fan-out-script path. The system prompt says "maximize parallel tool calls" and "for any other task — even one that would clearly benefit from parallelism — do NOT call this tool." The two are reconciled only by the fact that they govern different tools (ordinary tools vs. the Workflow tool).

Related skill text, `skill-workflow-authoring-reference.md`:

> A workflow structures work across many agents — to be comprehensive (**decompose and cover in parallel**), to be confident (independent perspectives and adversarial checks before committing), or to take on scale one context can't hold (migrations, audits, broad sweeps). **The script is where you encode that structure: what fans out, what verifies, what synthesizes.**

And the `parallel()` primitive's semantics:

> - `parallel(thunks: Array<() => Promise<any>>): Promise<any[]>` — run tasks concurrently. **This is a BARRIER: awaits all thunks before returning.** A thunk that throws (or whose agent errors) resolves to `null` in the result array — the call itself never rejects, so `.filter(Boolean)` before using the results. **Use ONLY when you genuinely need all results together.**

### 1g. Item 5: pushing AWAY from shell scripts toward native tools

Claude Code is the only agent here with a systematic, enumerated version of this. Stated reason, in `tool-description-bash-built-in-tools-note.md` (`ccVersion: "2.1.53"`):

> While the ${BASH_TOOL_NAME} tool can do similar things, **it's better to use the built-in tools as they provide a better user experience and make it easier to review tool calls and give permission.**

Note the stated reason is **UI/permission-review ergonomics, not efficiency or accuracy.**

The enumerated mapping, five separate bullets from the same version:

> Content search: Use ${GREP_TOOL_NAME} (NOT grep or rg)
> File search: Use ${GLOB_TOOL_NAME} (NOT find or ls)
> Read files: Use ${READ_TOOL_NAME} (NOT cat/head/tail)
> Edit files: Use ${EDIT_TOOL_NAME} (NOT sed/awk)
> Write files: Use ${WRITE_TOOL_NAME} (NOT echo >/cat <<EOF)
> Communication: Output text directly (NOT echo/printf)

The umbrella clause, `tool-description-bash-prefer-dedicated-tools.md` (`ccVersion: "2.1.71"`, reformatted as a bullet at `2.1.133`):

> **IMPORTANT: Avoid using this tool to run ${READ_ONLY_SEARCHING_BASH_COMMANDS} commands, unless explicitly instructed or after you have verified that a dedicated tool cannot accomplish your task. Instead, use the appropriate dedicated tool as this will provide a much better experience for the user:**

Note the escape hatch: "unless explicitly instructed **or after you have verified that a dedicated tool cannot accomplish your task**." Also note the trigger is specifically *read-only searching* commands — it does not forbid running arbitrary code.

**A one-word variant inside the Grep tool description** (from the wong2 gist, mid-2025):

> - If you need to identify/count the number of matches within files, use the Bash tool with `rg` (ripgrep) directly. **Do NOT use `grep`.**

An interesting inversion: here Bash *is* the preferred tool (because counting isn't supported by the Grep tool), but the specific forbidden binary is still named.

### 1h. A structured read-then-write batching recipe (the most copy-pasteable artifact here)

`system-reminder-memory-extraction-turn-budget.md`, `ccVersion: "2.1.173"`. This is the closest thing in the entire corpus to a worked example of *batching across turns*, and it motivates batching by an explicit turn budget:

> You have a limited turn budget. ${EDIT_TOOL_NAME} requires a prior ${READ_TOOL_NAME} of the same file, so **the efficient strategy is: turn 1 — issue all ${READ_TOOL_NAME} calls in parallel for every file you might update; turn 2 — issue all ${WRITE_TOOL_NAME}/${EDIT_TOOL_NAME} calls in parallel. Do not interleave reads and writes across multiple turns.**

Three techniques in one sentence: (i) the *dependency* (Edit requires prior Read) is stated, (ii) batching is justified by a **budget**, not a preference, (iii) an explicit anti-pattern is named ("do not interleave").

### 1i. The `/batch` slash command — fan-out as an orchestrated workflow

`agent-prompt-batch-slash-command.md`, `ccVersion: "2.1.281"`. Selected lines:

> You are orchestrating a large, parallelizable change across this codebase.

> 2. **Decompose into independent units.** Break the work into ${MIN_5_UNITS}–${MAX_30_UNITS} self-contained units. Each unit must:
>    - Be independently implementable in an isolated git worktree (no shared state with sibling units)
>    - Be mergeable on its own without depending on another unit's PR landing first
>    - Be roughly uniform in size (split large units, merge trivial ones)

> Once the plan is approved, spawn one background agent per work unit using the `${AGENT_TOOL_NAME}` tool. **All agents must use `isolation: "worktree"` and `run_in_background: true`. Launch them all in a single message block so they run in parallel.**

The **granularity floor and ceiling are numeric variables** (min/max unit counts, interpolated by the harness). That is a distinctive design: the batching instruction is paired with a target N.

The `/simplify` slash command, `ccVersion` from frontmatter, uses the same idiom:

> `/simplify → 4 cleanup agents in parallel → apply the fixes`
> ## Phase 1 — Review (4 cleanup agents in parallel)
> [launch] single message so they run concurrently.

---

## 2. OpenAI Codex CLI

**Kind: (c) OSS-SOURCE.** This is the strongest-sourced section in the report — Codex CLI is Apache-2.0 open source, so this is the vendor's actual shipped prompt, not a leak. I cloned `github.com/openai/codex` at HEAD dated **2026-09-30**.

### 2a. The clause

**File:** `codex-rs/core/gpt_5_2_prompt.md`, line 252 · **Where it sits:** the **`# Tool Guidelines` → `## Shell commands`** subsection — i.e. scoped to the shell, not stated globally.

> - When searching for text or files, prefer using `rg` or `rg --files` respectively because `rg` is much faster than alternatives like `grep`. (If the `rg` command is not found, then use alternatives.)
> - Do not use python scripts to attempt to output larger chunks of a file.
> - **Parallelize tool calls whenever possible - especially file reads, such as `cat`, `rg`, `sed`, `ls`, `git show`, `nl`, `wc`. Use `multi_tool_use.parallel` to parallelize tool calls and only this.**

Three features worth noting for prompt-copying purposes:

1. **"whenever possible"** — matches Claude Code's "Maximize use of parallel tool calls where possible to increase efficiency" almost exactly, minus the stated motivation.
2. **"especially file reads"** — an explicit *preference ordering* within the parallel set, with six concrete example commands enumerated.
3. **"and only this"** — a *monopoly* clause. Codex does not merely permit parallel calls; it names the single legal mechanism and forbids alternatives. This is unique in the corpus.
4. Codex states batching as a **recommendation** ("Parallelize ... whenever possible"), with no MUST and no dependency rule in this clause.

I confirmed by `diff` that this clause is present in `gpt_5_2_prompt.md` (GPT-5.2) and check the sibling prompt files; the parallelize line appears in `gpt_5_2_prompt.md` line 252. **NEEDS-CHECK:** I did not exhaustively confirm whether `gpt_5_codex_prompt.md` and `gpt-5.1-codex-max_prompt.md` carry the same line — my grep hit for "Parallelize" landed only on the 5.2 file, so it may be newer than the others. Treat as unverified for non-5.2 models.

### 2b. Codex's anti-shell clause (item 5)

Adjacent bullet, same file:

> - Do not use python scripts to attempt to output larger chunks of a file.

That is the entirety of Codex's anti-script guidance: a single prohibition, with **no stated reason**. Materially weaker than Claude Code's five-bullet enumeration with a UX rationale.

### 2c. Codex multi-agent prompt

**File:** `codex-rs/core/templates/collab/experimental_prompt.md` (labeled experimental). This is the **only** Codex parallel/agent prompt I found.

> ## Multi agents
> You have the possibility to spawn and use other agents to complete a task. For example, this can be use for:
> * Very large tasks with multiple well-defined scopes
> * When you want a review from another agent. This can review your own work or the work of another agent.
> * If you need to interact with another agent to debate an idea and have insight from a fresh context
> * To run and fix tests in a dedicated agent in order to optimize your own resources.
>
> **This feature must be used wisely. For simple or straightforward tasks, you don't need to spawn a new agent.**
>
> **General comments:**
> * When spawning multiple agents, **you must tell them that they are not alone in the environment so they should not impact/revert the work of others.**
> * Running tests or some config commands can output a large amount of logs. **In order to optimize your own context, you can spawn an agent and ask it to do it for you.** In such cases, **you must tell this agent that it can't spawn another agent himself (to prevent infinite recursion)**
> * When you're done with a sub-agent, don't forget to close it using `close_agent`.
> * Be careful on the `timeout_ms` parameter you choose for `wait_agent`. It should be wisely scaled.
> * Sub-agents have access to the same set of tools as you do so you must tell them if they are allowed to spawn sub-agents themselves or not.

This is the **conflict-safety** clause nobody else has: telling the fan-out children they are not alone in the environment so they don't clobber each other. And the context-protection motivation ("optimize your own context") is stated directly. Note Codex has **no `parallel()`-style script fan-out primitive** in its prompt — parallelism is achieved by *multiple spawn calls*, not by a self-written script.

Codex also has a wire-level `parallel_tool_calls: bool` field (`codex-rs/core/src/client_common.rs:31-32`, `client.rs:1001`) — an API flag, not prompt text. Flagged for completeness, not quoted as prompt text.

---

## 3. Cline

**Kind: (c) OSS-SOURCE.** Read from `github.com/cline/cline`, `sdk/packages/shared/src/prompt/system/act.ts` (and `yolo.ts`, which carries the same two clauses).

### The clause — the most aggressive imperative in the corpus

> - **You can call multiple tools in a single response. Before using tools, identify every independent read, search, command, or edit needed for the next step and emit all of those tool calls now, either as multiple tool calls or as one batched input for tools that accept arrays. Do not wait for one independent result before requesting another. Do not split independent reads, searches, checks, or edits across separate turns.**

Differences from every other agent, and the reason this is the most interesting artifact for the user's purpose:

- It is **procedural, not descriptive.** Three imperatives in sequence: enumerate → emit now → don't wait.
- **"emit all of those tool calls now"** — an explicit *planning step* ("identify every independent ... needed for the next step") precedes the calls. No other agent asks the model to enumerate first.
- **"or as one batched input for tools that accept arrays"** — this is the bridge to requirement (b) in the user's brief. Cline explicitly offers **two shapes in one clause**: N separate tool calls, *or* one array-shaped batched call to a single tool. No other agent in the corpus makes this two-shape offer.
- **"Do not wait for one independent result before requesting another"** — the anti-serialism rule stated as a prohibition on *waiting*.
- **"Do not split independent reads, searches, checks, or edits across separate turns."** — an enumerated anti-pattern, the only one of its kind found.

**The few-shot example block (item 6).** The very next bullet is Cline's parallelism examples, and it is the best few-shot parallelism list found anywhere:

> - **Good parallelism examples: read all known relevant files in one read_files call; run independent inspection commands in one run_commands call; emit independent read_files, search_codebase, and run_commands calls together in one response; emit multiple editor calls together when editing different files or non-overlapping regions.**

Note the third and fourth examples are the cross-tool and non-overlapping-region cases — the two hardest to teach. The second clause ("read all known relevant files in **one** read_files call") is again the batched-array shape rather than N calls.

**Where it sits:** both clauses are the **second-to-last and last** bullets of the `Remember:` list, immediately after "Always use absolute paths when referring to files" and immediately before the verification clause. Emphatic — three prohibitions in one bullet.

**NEEDS-CHECK:** I read only `act.ts` and `yolo.ts` in `sdk/packages/shared/src/prompt/system/`. I did not locate or read Cline's older/alternate system-prompt variants (the `apps/vscode/src/core/prompts/` equivalent), so a plan-mode or architect-mode Cline variant may carry different wording.

---

## 4. Aider

**Kind: (c) OSS-SOURCE.** Read from `github.com/aider-chat/aider` (cloned as `aider-ai-aider`).

**Finding: Aider ships NO tool-call batching clause.** Verified by:

- `rg -c -i "batch|parallel" aider/coders/base_prompts.py` → **no matches** (exit 1).
- `rg -n -i "one at a time|sequential" aider/coders/*.py` → three hits, all in `patch_coder.py`, and all are *code comments about the patch applier*, not prompt text.
- The only `PARALLEL` string in the whole repo is `TOKENIZERS_PARALLELISM` in `aider/help.py:138` — a HuggingFace env var, unrelated.

**NEEDS-CHECK / partial:** I searched `aider/coders/base_prompts.py` and the `*.py` prompt modules. Aider also ships YAML-based `.aider.conf.yml` / conventions files, but those are user-authored, not agent-shipped. I did not exhaustively read every `*_prompts.py`. I read the structure and the grep is clean on the main prompt module; I'm confident enough to report Aider as having no such clause, but flag it as a negative finding from targeted search rather than exhaustive read.

This is itself a data point for the synthesis: Aider's architecture is a strict edit-block protocol with a single `run` command per turn — the format structurally forecloses multi-call turns.

---

## 5. OpenHands / OpenDevin

**Kind: (c) OSS-SOURCE.** Read from `github.com/All-Hands-AI/OpenHands`.

**Finding: no batching clause in the system prompt; the parallelism lives in a client tool description.** `rg -n -i "parallel" src` produced only application/infra code and one prompt-bearing file.

`src/api/launch-child-conversation-client-tool.ts` — the `LAUNCH_CHILD_CONVERSATION_DESCRIPTION` constant:

> Start a NEW, independent conversation to work on a well-scoped task **in parallel with this one**, without the user having to run any CLI command. The new conversation is recorded as a child of this one.
>
> The child runs on its own. It does not block you, you do not see its output, and it cannot see this conversation's history — everything it needs must be in the task brief.
>
> Choosing target:
> * target="local" — runs on the same machine, in this conversation's workspace. Fast, no repository clone, no sandbox provisioning. Use this by default when the work is on code that is already checked out here.
> * target="cloud" — runs on OpenHands Cloud in its own isolated sandbox, from a git repository. Use this when the work should not touch the user's machine or when it needs a repository that is not checked out locally. ...
>
> Writing the task brief:
> * Put everything the child needs in "task": the goal, the relevant file paths, the constraints, the expected deliverable, and how it should report back.
> * **Keep each child's scope independent of its siblings so parallel children do not fight over the same files.**
> * **One call per delegated task. Do NOT call this tool twice for the same task.**

The clause **"Keep each child's scope independent of its siblings so parallel children do not fight over the same files"** is the sharpest conflict-avoidance rule in the corpus, and it's stated as a property of *the scope of each delegated unit* rather than as an isolation mechanism (unlike Claude Code's worktree isolation). Combined with **"One call per delegated task. Do NOT call this tool twice for the same task"** — which is an anti-duplication rule on the fan-out tool itself.

OpenHands also exposes a **settings toggle** labeled "Parallel tool calls" (`src/routes/agent-settings.tsx:348`, `src/mocks/settings-handlers.ts:108`) — a UI/transport-level switch, not prompt text.

**NEEDS-CHECK:** I did not locate OpenHands' system-prompt template files (the modern repo appears to have moved them; my `find` for `*.j2` and `microagents` returned nothing, and this is a large refactor-era tree). The main agent's system prompt may live in a Python package not present at this path or may be constructed at runtime. I am reporting what I actually read, and it does not include OpenHands' main system prompt.

---

## 6. Goose

**Kind: (c) OSS-SOURCE.** Read from `github.com/block/goose`, `crates/goose/src/prompts/`.

**Finding: NO parallel/batching clause.** Verified by reading every prompt file in that directory — `apps_create.md`, `apps_iterate.md`, `permission_judge.md`, `session_name.md`, `subagent_system.md`, `system.md`, `tiny_model_system.md` — and running `rg -n -i "parallel|batch|independent" .` across all of them: **zero matches.**

`system.md` in full is short and has no tool-discipline section at all:

> You are a general-purpose AI agent called goose, created by AAIF (Agentic AI Foundation).
> goose is being developed as an open-source software project.
> [# Extensions block — dynamic extension loading]
> # Response Guidelines
> Use Markdown formatting for all responses.

The closest Goose gets is in `subagent_system.md`, and it pushes the *opposite* direction — tool frugality:

> **Tool Efficiency Rules**:
> - Use the minimum number of tools needed to complete your task
> - Avoid exploratory tool usage unless explicitly required
> - Stop using tools once you have sufficient information
> - Provide clear, concise responses without excessive tool calls

> **Efficiency**: Use tools sparingly and only when necessary

This is a genuine **point of disagreement** worth surfacing: Goose's subagent prompt is explicitly anti-batching, while every other agent here is pro-batching. (d) paraphrase: Goose appears to treat token cost as the dominant constraint; the other agents treat turn-count/latency as the dominant constraint.

**NEEDS-CHECK:** I read the `crates/goose/src/prompts/` directory only. Goose's `/review` command has its own prompts (`crates/goose-cli/src/commands/review/default_review_prompt.md`, `orchestrator.rs`) that matched my `parallel` grep at the file level; I did not read their contents. There may be parallel language in the review orchestrator prompt. Flagged.

---

## 7. OpenCode

**Kind: (c) OSS-SOURCE — and unusually rich, because OpenCode vendors *many other agents' prompt files* in `packages/opencode/src/session/prompt/`.** Read from `github.com/sst/opencode`. This directory alone contains `anthropic.txt`, `codex.txt`, `gpt.txt`, `gpt-astra.txt`, `gemini.txt`, `kimi.txt`, `meta.txt`, `copilot-gpt-5.txt`, `default.txt`, `plan-mode.txt`, `plan-reminder-anthropic.txt`, `beast.txt`, `trinity.txt`, `build-switch.txt`, `plan.txt`.

**Caveat on attribution:** because OpenCode ships these as its own configurable agent presets, they are OpenCode's *shipped text* but several are clearly adapted from other vendors (the `anthropic.txt` parallel clause is byte-identical to Claude Code's). Treat each as "OpenCode's vendored variant of agent X's clause," not as an independent extraction of agent X.

### 7a. `anthropic.txt` (lines 83-84) — vendored Claude Code clause

> - You can call multiple tools in a single response. If you intend to call multiple tools and there are no dependencies between them, make all independent tool calls in parallel. Maximize use of parallel tool calls where possible to increase efficiency. However, if some tool calls depend on previous calls to inform dependent values, do NOT call these tools in parallel and instead call them sequentially. For instance, if one operation must complete before another starts, run these operations sequentially instead. Never use placeholders or guess missing parameters in tool calls.
> - If the user specifies that they want you to run tools "in parallel", you MUST send a single message with multiple tool use content blocks. For example, if you need to launch multiple agents in parallel, send a single message with multiple Task tool calls.

**`meta.txt` (lines 47-50) — the most explicitly *sectioned* treatment anywhere.** Under a dedicated heading `# Tool Use – Parallelism`:

> - You can call multiple tools "in parallel" by emitting separate messages, each with a tool call, in a single turn.
> - Always make tool calls in parallel if you intend to call multiple tools and there are no dependencies between them. Maximize use of parallel tool calls where possible to increase efficiency.
> - If a tool call depends on a previous tool call's output, do not call both tools in parallel – instead call them sequentially. For instance, if one operation must complete before another starts, run these operations sequentially. Never use placeholders or guess missing parameters in tool calls.

Three things to copy from this variant: (i) a **dedicated `Parallelism` heading** rather than burying it in a tool-usage policy; (ii) "**Always** make tool calls in parallel if..." — the strongest modal in the corpus short of MUST; (iii) an explicit **mechanism sentence** ("by emitting separate messages, each with a tool call, in a single turn") that tells the model *how* to batch, not just that it may. Note it says "separate messages ... in a single turn," which is a slightly different (and more precise) mechanism description than Claude Code's "single message with multiple tool calls."

`meta.txt` also has a **Local Computation** section that is directly relevant to the user's requirement (b) — pushing toward *inline* execution over script files:

> - For simple one-off Python computations, such as local file parsing, template rendering, or statistics computations, call `bash` with `python3 -c`. **Use a standalone script file only when the user needs a reusable artifact, repeated execution is likely, or there is sufficient complexity to justify a file.**
> - `read` may be used to inspect or locate files, but final numeric or rendered results should come from executed code, not copied text plus mental math.

And a `Task` section with a proactive-fan-out trigger:

> - You should proactively use the `Task` tool to launch specialized subagents when the task at hand can be easily split up into multiple parallel workers.
> - **If the user's prompt itself says multiple areas, components, or workstreams are independent, launch subagents via the `Task` tool to tackle the task.**
> - Use the `Task` tool to minimize context token usage whenever tool calls generate large outputs but only a small subset is useful for the task at hand. **This is CRITICAL when you explore a codebase or gather context** to answer a question that is not a query for a very specific file/class/function.

### 7b. `kimi.txt` line 13 — the most explicitly performance-framed clause

> **You have the capability to output any number of tool calls in a single response. If you anticipate making multiple non-interfering tool calls, you are HIGHLY RECOMMENDED to make them in parallel to significantly improve efficiency. This is very important to your performance.**

Note the different vocabulary: **"non-interfering"** rather than "independent," and **"HIGHLY RECOMMENDED"** rather than MUST. And the closing sentence is a pure motivation appeal with no external referent — the only clause in the corpus that argues batching is important *for the model's own sake*.

### 7c. `codex.txt` line 15 — one-line Codex variant, with the dependency rule

> - **Run tool calls in parallel when neither call needs the other's output; otherwise run sequentially.**

The most compact correct statement of the dependency rule I found anywhere: one sentence, no hedging, both branches stated.

### 7d. `gpt-astra.txt` — the terse harness style, and a hard subagent gate

> - Prefer parallelizing independent tool calls.

Three lines, in a `# Harness` bullet list, adjacent to the anti-shell-chain rule:

> - Prefer dedicated tools over shell commands; fall back to the shell when a tool cannot do what you need.
> - Do not chain shell commands with separators like `echo "====";` or `printf '---'`; the output becomes noisy in a way that makes the user's side of the conversation worse.

That anti-chaining clause with a **stated UX reason** is the item-5 pattern. And its subagent gate is the most restrictive found:

> # Delegation
> **Do not spawn subagents unless the user or applicable AGENTS.md/skill instructions explicitly ask for subagents, delegation, or parallel agent work.**

### 7e. `gpt.txt` line 6 — Codex's `multi_tool_use.parallel` with an added anti-chaining rule

> - Parallelize tool calls whenever possible - especially file reads. Use `multi_tool_use.parallel` to parallelize tool calls and only this. **Never chain together bash commands with separators like `echo "====";` as this renders to the user poorly.**

Same as Codex's own prompt, plus a "renders to the user poorly" reason. This is the **anti-shell-scripting** clause the brief asked about, in OpenCode's variant — the stated reason is again user-facing output quality, not efficiency.

### 7f. `copilot-gpt-5.txt` line 121 — parallel with a named exception

> If you think running multiple tools can answer the user's question, **prefer calling them in parallel whenever possible, but do not call semantic_search in parallel.**

**The only clause in the corpus that names a specific tool that must never be parallelized.** Worth copying: a blanket "parallelize" rule is easy to over-apply; naming the exception is a cheap safety valve.

### 7g. `plan-mode.txt` / `plan-reminder-anthropic.txt` — agent fan-out with numeric caps

> 2. **Launch up to 3 explore agents IN PARALLEL** (single message, multiple tool calls) to efficiently explore the codebase.
>  - Use 1 agent when the task is isolated to known files, the user provided specific file paths, or you're making a small targeted change.
>  - Use multiple agents when: the scope is uncertain, multiple areas of the codebase are involved, or you need to understand existing patterns before planning.
>  - **Quality over quantity - 3 agents maximum, but you should try to use the minimum number of agents necessary (usually just 1)**
>  - If using multiple agents: Provide each agent with a specific search focus or area to explore. Example: One agent searches for existing implementations, another explores related components, a third investigates testing patterns

> You can launch up to 1 agent(s) in parallel.

The parenthetical **"(single message, multiple tool calls)"** is a nice compact mechanism gloss. And the "Example: One agent ... another ... a third ..." is a genuine few-shot fan-out example (item 6). Note the same paragraph contains both a *cap* and a *discouragement* — the tension is resolved by "usually just 1."

### 7h. `agent.ts:184` — the subagent tool description

> description: `General-purpose agent for researching complex questions and executing multi-step tasks. Use this agent to execute multiple units of work in parallel.`

### 7i. `default.txt` line 72

> - Use the available search tools to understand the codebase and the user's query. **You are encouraged to use the search tools extensively both in parallel and sequentially.**

"Weakly encourages" — the mildest pro-batching language in the corpus. (d) note: the phrase "both in parallel and sequentially" reads more as permission than instruction, and is the weakest clause I found.

---

## 8. Roo Code / Kilo Code

**Kind: (c) OSS-SOURCE.** Read from `github.com/RooCodeInc/Roo-Code`, `src/core/prompts/sections/tool-use-guidelines.ts` (live source; I also confirmed the identical text in the committed Jest snapshots, so this is not a test artifact).

### The clause — notably *permissive*, and it contains an internal tension

> ```
> # Tool Use Guidelines
>
> 1. Assess what information you already have and what information you need to proceed with the task.
> 2. Choose the most appropriate tool based on the task and the tool descriptions provided. Assess if you need additional information to proceed, and which of the available tools would be most effective for gathering this information. For example using the list_files tool is more effective than running a command like `ls` in the terminal. It's critical that you think about each available tool and use the one that best fits the current step in the task.
> 3. **If multiple actions are needed, you may use multiple tools in a single message when appropriate, or use tools iteratively across messages. Each tool use should be informed by the results of previous tool uses. Do not assume the outcome of any tool use. Each step must be informed by the previous step's result.**
> ```

Read item 3 carefully, because it is doing something unusual. It grants permission ("you **may** use multiple tools in a single message when appropriate") and in the *same sentence* it imposes a sequential epistemics rule ("**Each tool use should be informed by the results of previous tool uses**... **Each step must be informed by the previous step's result**"). The second half is arguably in tension with true parallel batching — you cannot inform a call from the result of a call that hasn't returned. (d) paraphrase: this reads like a rule written for the serial case and only partially reconciled with the parallel case. For the user's purposes it is the *weaker* of the two designs, and the contrast with Cline's "Do not wait for one independent result before requesting another" is stark.

### Roo's tool-level parallel nudges

`src/core/prompts/tools/native-tools/read_file.ts:65`:

> Read a file and return its contents with line numbers for diffing or discussion. IMPORTANT: **This tool reads exactly one file per call. If you need multiple files, issue multiple parallel read_file calls.**

A tool-description-level batching hint of exactly the shape the user wants: *this tool is single-item, so batch by making N calls.*

`src/core/prompts/tools/native-tools/converters.ts:77-79` (comment, not prompt text — flagged as such):

> // Parallel tool calls are enabled by default. When parallelToolCalls is explicitly false,
> // we disable parallel tool use to ensure one tool call at a time.

**NEEDS-CHECK / not retrieved:** I did **not** clone `Kilo-Org/kilocode` successfully in this session (the clone produced no output I could verify, and I did not confirm the directory contents). Kilo Code is a Roo Code fork, so its text is likely near-identical to the above, but **I have not read Kilo Code's prompt text and am not quoting any.** Treat Kilo as unretrieved.

Also unretrieved: any Roo/Kilo rule-file or mode-specific variant beyond `tool-use-guidelines.ts`. Roo supports many modes; I read the shared section that the snapshots show is common to ask/architect/act modes, but did not diff every mode's system prompt.

---

## 9. Continue

**Kind: (c) OSS-SOURCE.** Read from `github.com/continuedev/continue`.

**Finding: NO system-prompt-level batching clause.** Grep across `core/` and `extensions/` for `in parallel|parallel tool` returned only application code and one tool constraint.

The only prompt-adjacent parallel language is a **negative constraint** on a single tool, `core/tools/definitions/editFile.ts:11`:

> "This tool CANNOT be called in parallel with any other tools, including itself"

— i.e. Continue ships an *exclusion* from parallelism, not an instruction to parallelize. (d) paraphrase: Continue's parallel handling is a transport-level feature (`extensions/cli/src/stream/streamChatResponse.helpers.ts:477` — "queue permissions (preserve order), then run approved tools in parallel") with a clever context-budget interaction in `extensions/cli/src/tools/readFile.ts:86-99` that divides per-call limits by the parallel call count and tells the model so:

> `(Note: limit reduced due to ${parallelCount} parallel tool calls. Single-tool limit: ... characters or ... lines.)`

That last one is a genuinely novel pattern worth noting: **dynamically injecting the current parallel fan-out width into a tool's description** so the model can budget its own reads. No other agent does this.

**NEEDS-CHECK:** I did not locate Continue's main system-prompt module (my `find` for `systemPrompts.ts` returned only the CLI's `systemMessage.ts`, which is about CLAUDE.md-style rules, not tool batching). Continue's rules may be primarily user-authored YAML. I searched but did not find a canonical agent system prompt to quote. Reporting as: no clause found in the paths I searched.

---

## 10. Synthesis

### 10a. Recurring phrasing patterns (quoted)

**Pattern 1 — "You can call multiple tools in a single response."** (capability grant)
Found verbatim in Claude Code (Piebald, kband, alireza), Cline, OpenCode's `anthropic.txt` and `meta.txt`. Codex paraphrases to "the capability to output any number of tool calls in a single response"; wong2's mid-2025 extraction says "the capability to call multiple tools in a single response." This is close to a fixed string across the Claude-derived family. **The "single response" framing is the near-universal unit of batching** — agents talk about composing one *response*, never one *turn* (except Cline, which uses "turn").

**Pattern 2 — "If you intend to call multiple tools and there are no dependencies between them…"** (the dependency gate)
Claude Code: "and there are no dependencies between them." OpenCode meta: "if you intend to call multiple tools and there are no dependencies between them." Codex: "when neither call needs the other's output." Cline: "identify every independent read, search, command, or edit." Kimi: "multiple non-interfering tool calls." **Four different words for the same concept: dependencies / neither needs the other's output / independent / non-interfering.** The "no dependencies" phrasing is the canonical form; the variants are not clearly better, but "independent" is the shortest and Cline is the only one to *enumerate the tool categories* it applies to.

**Pattern 3 — "Maximize use of parallel tool calls where possible to increase efficiency."** (maximization + motivation)
Byte-identical in Piebald, kband, alireza, and OpenCode's `anthropic.txt`. Codex: "Parallelize tool calls whenever possible." OpenCode meta: "**Always** make tool calls in parallel… Maximize use of parallel tool calls where possible to increase efficiency." Kimi: "HIGHLY RECOMMENDED… to significantly improve efficiency." Note: **only Claude Code names a motivation word ("efficiency") in the same sentence**; the modal strength varies from "whenever possible" (Codex) to "HIGHLY RECOMMENDED" (Kimi) to "Always" (meta) to "MUST" (the bash-specific and user-requested-agent variants).

**Pattern 4 — "For instance, if one operation must complete before another starts, run these operations sequentially instead."** (the negative example)
Present verbatim in Piebald, kband, alireza, OpenCode anthropic.txt, and OpenCode meta.txt (the last two with a typographic en-dash variant and the "Never use placeholders or guess missing parameters in tool calls" sentence appended). Codex's one-liner compresses it: "when neither call needs the other's output; otherwise run sequentially." **This single "if one operation must complete before another starts" example is the most replicated sentence in the entire corpus** — five sources, four independent authors.

**Pattern 5 — the `git status` / `git diff` example.** (concrete parallel example)
wong2 extraction, Claude Code's bash policy: "For example, if you need to run \"git status\" and \"git diff\", send a single message with two tool calls to run the calls in parallel." Codex's variant enumerates six: "`cat`, `rg`, `sed`, `ls`, `git show`, `nl`, `wc`." **Both name shell reads specifically, not MCP calls.** I found **no agent in this corpus that gives a parallel-tool example using an MCP tool.** For a user writing an MCP-focused clause, this is a gap in all the reference text.

**Pattern 6 — "Keep each child's scope independent of its siblings so parallel children do not fight over the same files."** (conflict safety)
OpenHands. Claude Code's analogue is mechanical rather than textual: `isolation: "worktree"`. Codex's analogue: "you must tell them that they are not alone in the environment so they should not impact/revert the work of others." **Three different mechanisms for the same hazard**: scope-drafting rule (OpenHands), tool parameter (Claude Code), instruction-to-the-child (Codex).

**Pattern 7 — context/token as motivation.** (item 7)
Claude Code: "Subagents are valuable for parallelizing independent queries **or for protecting the main context window from excessive results**." OpenCode meta: "Use the `Task` tool to minimize context token usage… This is CRITICAL when you explore a codebase." Codex: "Running tests or some config commands can output a large amount of logs. In order to optimize your own context, you can spawn an agent." Claude Code's memory-extraction reminder: "You have a limited turn budget… the efficient strategy is…". **Note the recurring two-part motivation: fan out to save context, and batch within a turn to save turns.** Only Claude Code states both in the same corpus.

### 10b. Points of disagreement

| Axis | Pro-batching camp | Anti / restrained camp |
|---|---|---|
| **Subagent fan-out** | Claude Code coordinator: "**Parallelism is your superpower**… Launch independent workers concurrently — don't serialize work" | Claude Code restraint: "Do not fan out multiple subagents on a single small task"; gpt-astra: "Do not spawn subagents unless the user… explicitly ask"; Codex: "For simple or straightforward tasks, you don't need to spawn a new agent" |
| **Tool frugality** | Claude Code: "Maximize use of parallel tool calls" | Goose subagent: "**Use the minimum number of tools needed**… Avoid exploratory tool usage… Stop using tools once you have sufficient information" |
| **Subagents vs. inline** | Claude Code: "Do the work inline when it is a small, bounded sub-task" | Claude Code (earlier): "When doing file search, prefer to use the Task tool in order to reduce context usage" — an older, broader delegation preference that the newer restraint clause explicitly narrows |
| **Serial vs. parallel epistemology** | Cline: "**Do not wait for one independent result before requesting another**" | Roo: "Each tool use should be informed by the results of previous tool uses… **Each step must be informed by the previous step's result**" |
| **Script fan-out** | Claude Code Workflow tool: `parallel()` / `pipeline()` primitives, "what fans out, what verifies, what synthesizes" | Claude Code *simultaneously*: "ONLY call this tool when the user has explicitly opted in… **For any other task — even one that would clearly benefit from parallelism — do NOT call this tool**" |

The sharpest disagreement is the last row. Claude Code ships both the most aggressive parallelization mandate in the corpus *and* a hard opt-in gate on the script-fan-out path. These are not contradictory because they govern different tools, but any clause the user writes that covers both shapes (as their brief requires) must resolve this tension explicitly — **the corpus contains no agent that has successfully merged a general parallel-mandate and a script-fan-out mandate in one clause.** Cline comes closest by merging the two *shapes* (N calls OR one array-batched call) but its array-batching is over its own `read_files`/`run_commands` tools, not an arbitrary self-written script.

### 10c. Notable one-offs

1. **Codex's "and only this"** — `Use multi_tool_use.parallel to parallelize tool calls and only this.` The only *monopoly* clause found. Names the mechanism and forbids all alternatives.
2. **Copilot-gpt-5's named exception** — "prefer calling them in parallel whenever possible, **but do not semantic_search in parallel**." The only clause naming a specific non-parallelizable tool.
3. **Continue's dynamic budget injection** — the parallel call count is written into the tool description at runtime: "limit reduced due to N parallel tool calls." The only self-sizing parallelism mechanism found.
4. **Cline's two-shape clause** — "either as multiple tool calls or as one batched input for tools that accept arrays." The only clause that offers both granularities in one sentence.
5. **Claude Code's numeric fan-out targets** — `/batch` interpolates `${MIN_5_UNITS}–${MAX_30_UNITS}`; plan-mode caps at 3 explore agents; `/simplify` uses exactly 4. **Numeric caps/floors on fan-out width are unique to Claude Code.**
6. **Kimi's "This is very important to your performance."** — the only motivation appeal addressed to the model's own performance with no external referent.
7. **Claude Code's `browser_batch` inverse tool** — `tool-description-browserbatch.md`: "Actions execute SEQUENTIALLY (not in parallel) and stop on the first error… **browser_batch cannot be nested.**" A tool whose *purpose* is round-trip batching but which is deliberately serial. Useful as a counterexample if the user's clause needs a boundary.
8. **Roo's internal tension** — grants multi-call permission and demands each step be informed by the previous step's result in the same numbered item.

---

## 11. Version-drift observations on Claude Code (relevant if the user copies old text)

From `Piebald-AI/claude-code-system-prompts/CHANGELOG.md` (lines quoted):

- **~line 2657** — "Tool Description: ReadFile — **Removed** the \"speculatively read multiple files in parallel\" guidance." So the old wong2-era Read-tool clause no longer ships.
- **~line 3500** — "Tool Description: Bash (Git commit and PR creation instructions) — simplified parallel command instructions; **removed \"You can call multiple tools in a single response\" preambles**; added GIT_COMMAND_PARALLEL_NOTE variable".
- **~line 1429** — "**REMOVED:** Tool Description: Bash command-chaining notes — Removes standalone Bash fragments for newline avoidance, **parallel Bash calls**, semicolon use, and `&&` chaining."
- **~line 1650** — "System Prompt: Coordinator mode orchestration — **Expands** the concurrency guidance: launch independent workers in parallel via multiple tool calls in one message and cover multiple research angles, **but don't parallelize simple tasks** that are faster in a single worker loop."
- **~line 1174** — "System Prompt: Subagent delegation restraint — **Limits subagent use to genuinely independent, sizeable, or parallel work**; keeps small tasks and inline verification in the parent agent; discourages redundant fan-out and duplicated work."

**Direction of travel:** the mid-2025 text is **more** pro-batching and more repetitive (the clause was duplicated into every tool description); the 2.1.x text is **more centralized** (one canonical paragraph under Tool Usage Policy) and **more balanced** (restraint clauses added, speculation removed). If the user wants the most current, most defensible wording, use the Piebald 2.1.30+ canonical paragraph plus the restraint section — not the wong2 gist.

---

## 12. Explicit list of what I could NOT find

1. **Codex CLI's non-5.2 prompt files** — I confirmed the "Parallelize tool calls whenever possible… `multi_tool_use.parallel`… and only this" clause in `gpt_5_2_prompt.md` line 252. I did **not** verify whether `gpt_5_codex_prompt.md`, `gpt-5.1-codex-max_prompt.md`, or `gpt-5.2-codex_prompt.md` carry it. NEEDS-CHECK.
2. **Codex's fan-out-script primitive** — I found no `parallel()`/`pipeline()`-style script orchestration in Codex's prompts, only the experimental multi-agent spawn prompt. If Codex has such a feature, I did not find its prompt text.
3. **Kilo Code** — clone not verified; **zero prompt text retrieved**. I am quoting nothing from Kilo.
4. **OpenHands' main system prompt** — not located. Its modern tree appears to have moved or runtime-constructed prompt templates. I only read `launch-child-conversation-client-tool.ts`. NEEDS-CHECK.
5. **OpenHands' microagent prompt files** — `find` for `*.j2` and `microagents` returned nothing at this path.
6. **Continue's main system prompt** — not located. I found no agent system-prompt module to quote; only the `editFile` non-parallel constraint and a runtime budget note.
7. **Goose's `/review` orchestrator prompt** — `crates/goose-cli/src/commands/review/default_review_prompt.md` and `orchestrator.rs` matched a `parallel` grep at file level; I did not read their contents. Goose's *main* prompts genuinely have zero matches, but the review path is unexamined.
8. **Cline's other prompt variants** — I read only `act.ts` and `yolo.ts` in `sdk/packages/shared/src/prompt/system/`. Plan/architect variants not retrieved.
9. **Aider's full prompt set** — negative finding from targeted grep of `base_prompts.py` and the `*_prompts.py` modules, not from an exhaustive read of every file. I believe Aider has no such clause but did not prove it exhaustively.
10. **Roo Code's per-mode prompts** — I read the shared `tool-use-guidelines.ts` section and confirmed it via snapshots; I did not diff every mode's assembled system prompt.
11. **Any agent's parallel-tool example using an MCP tool.** This is a real gap across the whole corpus, not just my searching. If the user needs an MCP-flavored few-shot example, they will have to write it.
12. **Gemini CLI** — not in scope of the request and not cloned; no text retrieved.
13. **Official vendor documentation (kind (a))** — I did **not** retrieve any Anthropic, OpenAI, or Blocks (Goose) documentation pages for this report. Everything above is either (b) leak/extract or (c) OSS source. Any claim about *official* guidance on parallel tool use is unverified.

---

## 13. Quick-reference: the six highest-value quotable clauses

For the user's stated purpose, ranked by direct applicability to "compose one turn with (a) concurrent discrete calls and (b) a self-written script that fans out":

1. **The general mandate (a)** — Claude Code, Piebald `ccVersion 2.1.30`, §1a above. Best single paragraph; contains capability + dependency rule + maximization + motivation.
2. **The two-shape clause (a + batched-array)** — Cline, §3. Only text found that offers N calls *or* one array-batched call in one sentence, with a procedural "identify → emit now" instruction and an anti-wait prohibition.
3. **The script-fan-out (b)** — Claude Code Workflow tool, §1f. The only real "self-written script that fans out" prompt found. Note the "ONLY… explicit opt-in" gate you'd need to reconcile.
4. **The mechanism gloss** — OpenCode `meta.txt` §7a: "by emitting separate messages, each with a tool call, in a single turn." Tells the model *how*, which most clauses do not.
5. **The budget-motivated two-turn recipe** — Claude Code memory-extraction reminder, §1h. "turn 1 — all reads in parallel; turn 2 — all writes in parallel. Do not interleave."
6. **The named exception** — Copilot-gpt-5 via OpenCode, §7f: "prefer calling them in parallel whenever possible, but do not semantic_search in parallel." A cheap guard against over-application.
