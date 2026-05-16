# Prompt Comparison Report: nullslop vs pi-mono

**Goal**: Identify why nullslop doesn't update test code or run `cargo test` when implementing features, while pi consistently does.

## Executive Summary

The root cause is a combination of **minimal tool descriptions**, **no per-tool behavioral guidelines in the system prompt**, and **no "Available tools" enumeration** in the system prompt text. These three gaps mean the LLM receives less explicit guidance about its capabilities and how to use them.

---

## 1. System Prompt Structure Comparison

### pi-mono

```
You are an expert coding assistant operating inside pi, a coding agent harness.
You help users by reading files, executing commands, editing code, and writing new files.

Available tools:
- read: Read file contents
- bash: Execute bash commands (ls, grep, find, etc.)
- edit: Make precise file edits with exact text replacement, including multiple disjoint edits in one call
- write: Create or overwrite files

In addition to the tools above, you may have access to other custom tools depending on the project.

Guidelines:
- Use bash for file operations like ls, rg, find
- Use edit for precise changes (edits[].oldText must match exactly)
- When changing multiple separate locations in one file, use one edit call with multiple entries in edits[] instead of multiple edit calls
- Each edits[].oldText is matched against the original file, not after earlier edits are applied. Do not emit overlapping or nested edits. Merge nearby changes into one edit.
- Keep edits[].oldText as small as possible while still being unique in the file. Do not pad with large unchanged regions.
- Use read to examine files instead of cat or sed.
- Use write only for new files or complete rewrites.
- Be concise in your responses
- Show file paths clearly when working with files
```

### nullslop (active persona at `~/.config/nullslop/personas/coding-assistant.md`)

```
You are an expert coding assistant. You help users by reading files,
executing commands, editing code, and writing new files.

Guidelines:
- Use bash for file operations like ls, rg, find
- Be concise in your responses
- Show file paths clearly when working with files
```

### Key Differences

| Aspect | pi-mono | nullslop |
|---|---|---|
| **"Available tools" section** | Yes — enumerates tools with one-line descriptions | No — tools only exist as API tool definitions |
| **Tool-specific guidelines** | Yes — dynamically injected per tool | No — static persona only |
| **Identity framing** | "...operating inside pi, a coding agent harness" | No harness framing |
| **Custom tools mention** | "you may have access to other custom tools" | Not mentioned |

---

## 2. Tool Description Comparison (sent via API)

The tool definitions sent to the LLM via the provider API differ significantly in detail level.

### bash

**pi-mono**:
> Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last 2000 lines or 50KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds.

**nullslop**:
> Execute a bash command in the current working directory. Returns stdout and stderr. Optionally provide a timeout in seconds.

### read

**pi-mono**:
> Read the contents of a file. Supports text files and images (jpg, png, gif, webp). Images are sent as attachments. For text files, output is truncated to 2000 lines or 50KB (whichever is hit first). Use offset/limit for large files. When you need the full file, continue with offset until complete.

**nullslop**:
> Read the contents of a file. Use offset and limit for large files.

### edit

**pi-mono**:
> Edit a single file using exact text replacement. Every edits[].oldText must match a unique, non-overlapping region of the original file. If two changes affect the same block or nearby lines, merge them into one edit instead of emitting overlapping edits. Do not include large unchanged regions just to connect distant changes.

**nullslop**:
> Edit a file using exact text replacement. Each oldText must match a unique, non-overlapping region of the original file. Returns a unified diff of the changes.

### write

**pi-mono**:
> Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories.

**nullslop**:
> Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories.

*(write descriptions are essentially identical)*

---

## 3. Per-Tool Prompt Guidelines — The Biggest Structural Gap

pi-mono has a **`promptGuidelines`** field on each tool definition that injects behavioral instructions into the system prompt. nullslop has no equivalent mechanism.

**pi-mono tool guidelines injected into the system prompt:**

| Tool | Guidelines |
|---|---|
| **edit** | "Use edit for precise changes (edits[].oldText must match exactly)" |
| **edit** | "When changing multiple separate locations in one file, use one edit call with multiple entries in edits[] instead of multiple edit calls" |
| **edit** | "Each edits[].oldText is matched against the original file, not after earlier edits are applied. Do not emit overlapping or nested edits. Merge nearby changes into one edit." |
| **edit** | "Keep edits[].oldText as small as possible while still being unique in the file. Do not pad with large unchanged regions." |
| **read** | "Use read to examine files instead of cat or sed." |
| **write** | "Use write only for new files or complete rewrites." |

pi-mono also has **`promptSnippet`** — a one-line description shown in the "Available tools" section:

| Tool | Snippet |
|---|---|
| **bash** | "Execute bash commands (ls, grep, find, etc.)" |
| **read** | "Read file contents" |
| **edit** | "Make precise file edits with exact text replacement, including multiple disjoint edits in one call" |
| **write** | "Create or overwrite files" |

nullslop's `ToolDefinition` struct only has `name`, `description`, and `parameters`. There is no `prompt_snippet` or `prompt_guidelines` field.

---

## 4. How pi-mono Dynamically Builds the System Prompt

pi-mono iterates over registered tools and collects both `promptSnippet` and `promptGuidelines` from each tool definition. These are then:

1. **`promptSnippet`** → listed in the "Available tools" section of the system prompt
2. **`promptGuidelines`** → appended as bullet points under "Guidelines"

This means the system prompt adapts based on which tools are active. If a custom tool provides guidelines, they appear automatically.

### Source: `agent-session.ts`

```typescript
const toolSnippets: Record<string, string> = {};
const promptGuidelines: string[] = [];

for (const name of validToolNames) {
    const snippet = this._toolPromptSnippets.get(name);
    if (snippet) {
        toolSnippets[name] = snippet;
    }
    const toolGuidelines = this._toolPromptGuidelines.get(name);
    if (toolGuidelines) {
        promptGuidelines.push(...toolGuidelines);
    }
}
```

### Source: `system-prompt.ts`

```typescript
// Tool snippets → "Available tools" section
const toolsList = visibleTools
    .map((name) => `- ${name}: ${toolSnippets![name]}`)
    .join("\n");

// Guidelines → "Guidelines" section
for (const guideline of promptGuidelines ?? []) {
    addGuideline(normalized);
}
```

nullslop has no equivalent pipeline. The system prompt is assembled from: skills block → pinned system entries → persona + env context. Tool definitions go to the provider API but contribute nothing to the system prompt text.

---

## 5. Root Cause Analysis

### Why nullslop doesn't update test code or run tests

The LLM's behavior is driven by what it sees in the system prompt and tool definitions. Here's the chain:

1. **No "Available tools" section** — The LLM doesn't see a clear enumeration of its tools in the system prompt. It relies solely on the tool definitions sent via the API, which some models weight less heavily than system prompt text.

2. **Minimal tool descriptions** — nullslop's bash description is 2 sentences. pi-mono's is 3 sentences with specific details about output truncation and temp files. Richer descriptions give the LLM more confidence about what the tool can do.

3. **No tool-specific behavioral guidelines** — pi-mono injects guidelines like "Use bash for file operations like ls, rg, find" directly into the system prompt. This primes the LLM to think of bash as a general-purpose tool. nullslop's persona has this guideline, but it's the only bash-related instruction — there's nothing about running tests, checking compilation, etc.

4. **Static persona vs. dynamic prompt** — pi-mono's system prompt is dynamically assembled from tool contributions. nullslop's persona is a static markdown file. If a tool has best practices (like "run tests after changes"), pi-mono can encode that in the tool definition; nullslop can't.

### Impact Ranking

| Factor | Impact | Effort to Fix |
|---|---|---|
| Add "Available tools" section to system prompt | High | Low |
| Enrich tool descriptions with more detail | Medium | Low |
| Add `prompt_guidelines` field to `ToolDefinition` | High | Medium |
| Add `prompt_snippet` field to `ToolDefinition` | Medium | Medium |
| Inject tool guidelines into system prompt assembly | High | Medium |

---

## 6. Recommendations

### Quick Wins (Low Effort)

1. **Add "Available tools" section to the persona** — Manually enumerate tools in the persona body or env_context builder. This gives the LLM an explicit capability list without any structural changes.

2. **Enrich tool descriptions** — Expand the `description` field on `ToolDefinition` for bash, read, and edit to match pi-mono's level of detail. No code changes needed beyond the description strings.

### Structural Fixes (Medium Effort)

3. **Add `prompt_guidelines` to `ToolDefinition`** — Add a `Vec<String>` field that carries behavioral instructions per tool. Each tool (bash, read, edit, write) would register its own guidelines.

4. **Inject tool guidelines into system prompt assembly** — In `assembly.rs` (or `env_context.rs`), collect `prompt_guidelines` from all registered tool definitions and append them to the "Guidelines" section of the system prompt.

5. **Add `prompt_snippet` to `ToolDefinition`** — A one-line summary per tool for the "Available tools" section.
