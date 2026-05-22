# nullslop-bench

Harness benchmarking suite for the nullslop agent pipeline. Runs programmatic tasks through the full actor system, captures per-task statistics, and compares results across code changes.

## Usage

All commands are run through the justfile recipe:

```bash
# Run all tasks against multiple models
just bench run --model openai/gpt-4o --model anthropic/claude-sonnet-4

# Run only specific tasks
just bench run --model openai/gpt-4o --only hello-world,json-parser

# Exclude specific tasks
just bench run --model openai/gpt-4o --exclude fix-syntax-broken-python

# View results as a formatted table
just bench show bench-results/2026-05-21T10-30-00/results.csv

# Compare two runs (delta table with ±deltas)
just bench compare bench-results/run-a/results.csv bench-results/run-b/results.csv
```

## Subcommands

### `run`

Runs the full bench suite. For each task × model pair, the runner:

1. Copies fixtures into an isolated working directory
2. Spins up a fresh actor system with the task's tool set
3. Switches to the requested model
4. Sends messages sequentially (waits for session idle between each)
5. Runs the task's verification function against the working directory
6. Writes one CSV row with stats

**Flags:**

| Flag | Description |
|------|-------------|
| `--model` | Model ID (repeatable). Must match a provider ID in the user's config. |
| `--db` | Database path (currently unused — each run gets an isolated in-memory DB). |
| `--only` | Comma-separated task names to include. |
| `--exclude` | Comma-separated task names to skip. |

### `show`

Reads a bench CSV and renders a formatted terminal table:

```
just bench show bench-results/2026-05-21T10-30-00/results.csv
```

### `compare`

Reads two bench CSVs, matches rows by (task name, model), and renders a diff table with per-cell deltas. Cells are colored green (improvement) or red (regression):

```
just bench compare baseline.csv new.csv
```

## CSV Schema

Each row represents one task × model pair:

| Column | Type | Description |
|--------|------|-------------|
| `name` | string | Task name |
| `model` | string | Model ID |
| `turns` | int | Number of user + assistant turns |
| `tokens_in` | int | Prompt tokens sent |
| `tokens_out` | int | Completion tokens received |
| `cost` | float | Total cost in USD |
| `wall_time_ms` | int | Wall-clock time in milliseconds |
| `passed` | bool | Whether the verification function succeeded |
| `status` | string | `completed` or `timeout` |

## Task Categories

### 1-shot tasks

Single message, model produces output from scratch with tool access.

| Task | Description | Verification |
|------|-------------|-------------|
| `hello-world` | Write and run a Rust hello world | Compiles |
| `json-parser` | Parse a JSON file of people, print name + age | Compiles + input.json exists |
| `word-frequency` | Count word frequencies from a text file, print top 5 | Compiles + input.txt exists |
| `http-server` | Write a minimal HTTP server (compile-check only) | Compiles |
| `markdown-to-html` | Convert markdown to HTML, read/write from files | Compiles + output.html exists |

### Fix-code tasks

Model receives broken code and must diagnose and fix it.

| Task | Bug type | Description | Verification |
|------|----------|-------------|-------------|
| `fix-syntax-broken-rust` | Syntax | Missing semicolon in for-loop body | Compiles + runs |
| `fix-syntax-broken-python` | Syntax | Missing closing paren + `=` instead of `==` | Runs and prints fib sequence |
| `fix-logic-fizzbuzz` | Logic | FizzBuzz check order — "FizzBuzz" is unreachable | Prints "FizzBuzz" for 15 |
| `fix-logic-sort` | Logic | Off-by-one in inner loop → index out of bounds | Sorts and prints correctly |

### Redirect tasks

Multi-turn: first message asks for X, second message says "actually, do Y instead". Verification confirms Y was done.

| Task | First ask | Redirect | Verification |
|------|-----------|----------|-------------|
| `redirect-change-color` | Change background to red | Actually make it dark gray, heading orange | BG is #333, heading is orange |
| `redirect-refactor-function` | Add volume calculation | Remove volume, add paint estimation instead | Has "paint", no "volume" |
| `redirect-switch-language` | Add word counting to Python | Rewrite the whole thing in Rust | Rust compiles, reads input.txt |

## Adding New Tasks

1. **Add a fixture directory** (if needed) under `fixtures/<task-name>/` with the starting files.
2. **Add a `BenchTask` entry** in `src/tasks.rs` with:
   - `name` — unique kebab-case identifier
   - `messages` — one string per turn
   - `fixture_dir` — `Some("fixture-dir-name")` or `None`
   - `timeout` — generous wall-clock timeout
   - `tools` — which builtins to enable
   - `verify` — a `fn(&Path) -> bool` that checks the working directory
3. **Write the verification function** in the same file. Test observable behavior only: files exist, programs compile, programs produce correct output.

## Output

Results are written to `bench-results/<timestamp>/results.csv`. Each task's working directory is preserved at `bench-results/<timestamp>/<task>/<model>/` for manual inspection after the run.
