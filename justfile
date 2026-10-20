# sccache (optional): when installed, cargo compiles through it so dependency
# crates are served from its cache instead of being recompiled. Builds fall
# back to plain rustc when sccache is missing, so it is never required. A
# caller-provided RUSTC_WRAPPER (set, or explicitly empty to disable) wins.
export RUSTC_WRAPPER := env_var_or_default('RUSTC_WRAPPER', `command -v sccache 2>/dev/null || true`)

# Default per-test timeout (seconds) for every #[rstest::rstest] test.
# rstest reads this at COMPILE TIME (the proc macro bakes Duration::from_secs(N)
# into the generated test), so it must be present whenever rustc runs — hence an
# export here, and a mirror in .cargo/config.toml [env] for entrypoints that
# bypass just (rust-analyzer, bare cargo in CI). Explicit #[timeout(...)] on a
# test overrides this default; use that only when a test genuinely needs >10s.
export RSTEST_TIMEOUT := "10"

COPYRIGHT_NAME := "Jayson Lennon"
COPYRIGHT_YEAR := "2026"

# Stage everything (dotfiles included) and commit (one-line message)
commit MSG:
    git add -A && git commit -q -m "{{MSG}}"

# One suite run: fail-fast disabled, full output captured to target/test-output.log
# with a summary and failing test names — details via `just test-failures`, never re-runs.
[doc("Run the workspace test suite once, capturing full output to target/test-output.log.")]
test:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p target
    # One suite run; full output captured so failure details never require a
    # re-run. tee must not mask cargo's exit code (pipefail handles it).
    set -o pipefail
    if ! cargo test --workspace --no-fail-fast 2>&1 | tee target/test-output.log; then
        TEST_STATUS="failed"
    else
        TEST_STATUS="passed"
    fi
    echo ""
    echo "==> Test summary ($(date -u +%Y-%m-%dT%H:%M:%SZ)) — full log: target/test-output.log"
    awk '/^test result:/ {ok+=$4; fail+=$6} END {printf "    passed: %d  failed: %d\n", ok, fail}' target/test-output.log
    if [ "$TEST_STATUS" = "failed" ]; then
        echo "    Failing tests (details: just test-failures):"
        awk '/^failures:$/{f=1; next} f && /^    /{gsub(/^    /,""); print "      - " $0; found=1; next} f && NF && !/^    /{f=0} END{if(!found) print "      (no named failures in log; check test-output.log)"}' target/test-output.log | sort -u
        exit 1
    fi

# Show failing tests from the last `just test` run WITHOUT re-running anything.
test-failures:
    #!/usr/bin/env bash
    set -euo pipefail
    LOG=target/test-output.log
    if [ ! -f "$LOG" ]; then
        echo 'No test log found at target/test-output.log — run `just test` first.' >&2
        exit 1
    fi
    if grep -qE '^test result: FAILED|^error: could not compile' "$LOG"; then
        grep -E '^failures:$' -A 100 "$LOG" | grep -E '^    [a-z_]+' | sed 's/^    //' | sort -u
    else
        echo "Last run passed — no failures in $LOG."
    fi

# Sanctioned iterate-on-failure loop: fix code, re-run `just test-one <name>`,
# then confirm with a final `just test` before committing.
[doc("Run only tests matching FILTER across the whole workspace.")]
test-one FILTER:
    cargo test --workspace --no-fail-fast {{FILTER}}

# Fast compile check of the whole workspace (no codegen).
check:
    cargo check --workspace

# Report release-binary size and per-crate .text breakdown
size-report:
    #!/usr/bin/env bash
    set -euo pipefail

    BIN=target/release/jinn
    cargo build --release

    echo "==> Binary: $BIN"
    ls -la "$BIN" | awk '{printf "    %s bytes\n", $5}'
    du -h "$BIN" | awk '{print "    " $1 " on disk"}'

    echo '==> Section table (largest first):'
    size -A "$BIN" | tail -n +3 | sort -k2 -n -r | head -12 | awk '{printf "    %-22s %10d bytes\n", $2, $3}' || true

    if command -v cargo-bloat >/dev/null 2>&1; then
        echo '==> Top crates by .text (cargo-bloat):'
        cargo bloat --release -n 15 --crates 2>/dev/null | tail -n +3 || true
    else
        echo '    (cargo-bloat not installed; skipping per-crate breakdown)'
    fi

    if command -v upx >/dev/null 2>&1; then
        TMP="$(mktemp)"
        cp "$BIN" "$TMP"
        if upx -q "$TMP" >/dev/null 2>&1; then
            UPX_SIZE=$(stat -c %s "$TMP")
            RAW_SIZE=$(stat -c %s "$BIN")
            echo '==> UPX-packed estimate:'
            awk -v r="$RAW_SIZE" -v p="$UPX_SIZE" 'BEGIN{printf "    %d bytes (%.1f%% of raw)\n", p, p*100.0/r}'
        fi
        rm -f "$TMP"
    else
        echo '    (upx not installed; skipping packed-size estimate)'
    fi

# Run clippy across the workspace.
clippy:
    cargo clippy --workspace --all-targets

# Check formatting without writing changes.
fmt:
    cargo fmt -- --check

# Apply formatting fixes.
fmt-fix:
    cargo fmt

# Run all linters (check + clippy + fmt check + test-attr guard + portable-path guard)
lint:
    cargo check --workspace
    just clippy
    cargo fmt -- --check
    just lint-testattr
    just lint-portable-path

[doc("Fail on hardcoded absolute path literals, std::env::temp_dir, or path-shaped test fixtures that only pass on one platform.")]
lint-portable-path:
   #!/usr/bin/env python3
   import os
   import re
   import sys

   # Each entry: relative path -> why the absolute literal is legitimate there.
   ALLOWLIST = {
       "crates/jinn-mcp/tests/excalimate_live.rs":
           "its /tmp/excalimate/... default names where the OPTIONAL excalimate "
           "server binary is installed; nothing creates it at runtime, so a "
           "tempdir root would point the skip-if-absent lookup at a path that "
           "does not exist",
       "crates/jinn-common/src/app_paths.rs":
           "asserts the POSIX system-data layout (/usr/share/jinn) as the "
           "marker distinguishing the platform layout from the JINN_PATHS_ROOT "
           "override layout; the value is the subject under test, not a fixture",
       "crates/jinn-common/src/path_display.rs":
           "uses /opt/jinn/config.toml as a path that is guaranteed to lie "
           "outside the home directory on every platform, so shorten_path has "
           "nothing to compress; the path is the subject under test",
       "crates/jinn-mcp/src/transport.rs":
           "uses /opt/jinn/bin/jinn as a synthetic executable path for the "
           "<jinn> token expansion; the path never touches the filesystem",
       "crates/slices/jinn-chat-input-msg/src/file_picker_state.rs":
           "uses /etc as a filter string and /proj as a synthetic cwd to "
           "exercise absolute-filter resolution; neither path is opened, and "
           "the assertion is about the filter string passing through",
       "crates/slices/jinn-stream-rules/src/rules.rs":
           "interpolates a toml_edit::Value into a TOML document; the Value "
           "carries the path as an already-escaped TOML string, so the "
           "interpolation is safe by construction",
   }
   SKIP_DIRS = {"target", "vendor"}

   # A double-quoted string starting with /tmp or /home (bare "/tmp" included),
   # or the exact std::env::temp_dir token (not the substring temp_dir).
   OFFENSE = re.compile(r'"/(?:tmp|home)(?:[/"]|$)|std::env::temp_dir')

   # Shape (a): a raw-string JSON/TOML literal with a {placeholder} in it.
   RAW_LITERAL = re.compile(r'r#"(.*?)"#', re.S)
   PLACEHOLDER = re.compile(r"\{([A-Za-z_][A-Za-z0-9_]*)\}")
   # A placeholder whose name reads as a path, or a literal that carries a
   # path-shaped key. Both mean a filesystem path is being pasted into a
   # string literal, where a Windows path's backslashes are an invalid
   # JSON/TOML escape.
   PATHISH_NAME = re.compile(
       r"^(path|dir|file|cwd|workdir|target|src|dest|destination|location|"
       r"root|base_dir|basedir|project|repo|home|"
       r".*_path|.*_dir|.*_file)$",
       re.I,
   )
   PATH_KEY = re.compile(r'"(?:path|dir|file|cwd|workdir|location)"\s*:')
   # A placeholder that is a UUID, a session id, a message, or a JSON
   # fragment is not a filesystem path; it cannot carry a backslash.
   NOT_A_PATH = re.compile(
       r"^(id|uuid|guid|session_id|user_id|msg|message|text|body|content|"
       r"name|title|label|desc|description|prompt|error|reason|status|"
       r"kind|type|script|command|cmd|args|json|value|key|branch|"
       r".*_id|.*_uuid|.*_msg|.*_text|.*_name)$",
       re.I,
   )
   # A path-shaped key matters only when a placeholder supplies its value.
   # A literal value ("dir":"wt-b") is already a valid JSON string. Both a
   # named placeholder ("path": "{path}") and a positional one ("path": "{}")
   # can carry a filesystem path into the literal.
   KEY_WITH_PLACEHOLDER = re.compile(
       r'"(?:path|dir|file|cwd|workdir|location)"\s*:\s*"[^"]*'
       r"(?:\{[A-Za-z_][A-Za-z0-9_]*\}|\{\})"
   )

   # Shape (b): a test expectation built from a path's display form in a
   # file that renders paths through a home-compressing production function.
   #
   # The platform temp directory is under the user profile on Windows, so a
   # temp root is not outside home there — it is on Unix. A test whose intent
   # is "a path outside home" must not derive its expectation from
   # `Path::display`, because production renders such a path tilde-compressed
   # there and uncompressed here.
   #
   # The signature is the conjunction, not either half: the display-form
   # expectation on one side, the home-compressing call on the other. A file
   # with only one of them is legitimate — a display-form expectation with no
   # compression in play, or compression with an expectation that already
   # accounts for it.
   DISPLAY_EXPECT = re.compile(
       r"let\s+\w*(?:expected|label|cwd_label|path)\w*\s*(:\s*String)?\s*=\s*"
       r"[^;]*\.display\(\)"
   )
   HOME_COMPRESS = re.compile(
       r"shorten_path\s*\(|display_label\s*\(|tilde_home\s*\(|"
       r"render_session_cwd_line\s*\("
   )

   # Shape (c): a POSIX absolute path hardcoded inside an assertion.
   POSIX_ABS = re.compile(r'assert[^;]*"/(?:etc|usr|var|opt|home|tmp|bin|sbin)')

   offenders = []
   for dirpath, dirnames, filenames in os.walk(os.getcwd()):
       dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
       for fn in filenames:
           if not fn.endswith(".rs"):
               continue
           fpath = os.path.join(dirpath, fn)
           # Normalize to forward slashes so the ALLOWLIST keys are
           # platform-independent.
           rel = os.path.normpath(os.path.relpath(fpath, os.getcwd())).replace(
               os.sep, "/"
           )
           if rel in ALLOWLIST:
               continue
           with open(fpath, encoding="utf-8") as f:
               src = f.read()
           lines = src.splitlines()
           for ln, line in enumerate(lines, start=1):
               m = OFFENSE.search(line)
               if m:
                   offenders.append((rel, ln, "hardcoded-path", m.group(0)))
           for m in RAW_LITERAL.finditer(src):
               body = m.group(1)
               names = set(PLACEHOLDER.findall(body))
               if not names:
                   continue
               lineno = src[: m.start()].count("\n") + 1
               pathish = sorted(
                   n
                   for n in names
                   if PATHISH_NAME.match(n) and not NOT_A_PATH.match(n)
               )
               keyish = bool(KEY_WITH_PLACEHOLDER.search(body))
               if pathish or keyish:
                   hit = ", ".join(pathish) if pathish else "path key"
                   offenders.append((rel, lineno, "unescaped-path-in-literal", hit))
           for idx, line in enumerate(lines):
               if not DISPLAY_EXPECT.search(line):
                   continue
               if HOME_COMPRESS.search(src):
                   offenders.append(
                       (rel, idx + 1, "display-expect-where-home-compressed", "")
                   )
           for ln, line in enumerate(lines, start=1):
               m = POSIX_ABS.search(line)
               if m:
                   offenders.append((rel, ln, "posix-absolute-in-assertion", m.group(0)))

   for rel, ln, kind, hit in offenders:
       print(f"ERROR: {rel}:{ln}: {kind}: {hit}", file=sys.stderr)
   if offenders:
       print(
           f"\n{len(offenders)} site(s) build a path fixture that only holds on\n"
           "one platform:\n"
           "  - a path pasted into a JSON/TOML string literal is invalid on\n"
           "    Windows (its backslashes are an invalid escape) — build the\n"
           "    document with serde_json::json! or toml::Value instead;\n"
           "  - the platform temp directory is under the user profile on\n"
           "    Windows, so a temp root is NOT outside home there — use a\n"
           "    path rooted at the filesystem root, or assert the property\n"
           "    (shorten_path) rather than a literal;\n"
           "  - a POSIX absolute path in an assertion only holds on Unix —\n"
           "    derive the expectation from the platform's own path rules.\n"
           "In tests, build paths from a tempfile root:\n"
           '  let dir = tempfile::tempdir().expect("temp dir");\n'
           '  let path = dir.path().join("name");\n'
           "and derive BOTH sides of each assertion from the same expression.\n"
           "(If a literal is genuinely legitimate, add the file to ALLOWLIST\n"
           "in the lint-portable-path recipe with a stated reason.)",
           file=sys.stderr,
       )
       sys.exit(1)

[doc("Fail on bare test attributes missing their rstest companion.")]
lint-testattr:
   #!/usr/bin/env python3
   import os
   import re
   import sys

   ALLOWLIST = set()
   SKIP_DIRS = {"target", "vendor"}

   def is_test_attr(s):
       if re.fullmatch(r"#\[test\]", s):
           return True
       return bool(re.fullmatch(r"#\[tokio::test(?:\([^()]*(?:\([^()]*\)[^()]*)?\))?\]", s))

   offenders = []
   for dirpath, dirnames, filenames in os.walk(os.getcwd()):
       dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
       for fn in filenames:
           if not fn.endswith(".rs"):
               continue
           fpath = os.path.join(dirpath, fn)
           rel = os.path.normpath(os.path.relpath(fpath, os.getcwd()))
           if rel in ALLOWLIST:
               continue
           with open(fpath, encoding="utf-8") as f:
               lines = f.readlines()
           i, n = 0, len(lines)
           while i < n:
               s = lines[i].strip()
               if s.startswith("#["):
                   # Walk the contiguous attribute region (multi-line ok).
                   depth = 0
                   has_rstest = False
                   has_bare = False
                   bare_line = None
                   j = i
                   while j < n:
                       sj = lines[j].strip()
                       depth += sj.count("[") + sj.count("(")
                       depth -= sj.count("]") + sj.count(")")
                       if "rstest" in sj:
                           has_rstest = True
                       if depth == 0 and is_test_attr(sj):
                           has_bare = True
                           bare_line = j + 1
                       if depth <= 0:
                           nxt = j + 1
                           if nxt < n and lines[nxt].strip().startswith("#["):
                               j = nxt
                               continue
                           break
                       j += 1
                   else:
                       j = n - 1
                   if has_bare and not has_rstest:
                       offenders.append((rel, bare_line))
                   i = j + 1
               else:
                   i += 1

   for rel, ln in offenders:
       print(f"ERROR: {rel}:{ln}: bare test attribute lacks an #[rstest::rstest] companion", file=sys.stderr)
   if offenders:
       print(
           f"\n{len(offenders)} test(s) would run WITHOUT the rstest timeout.\n"
           "Stack #[rstest::rstest] above the attribute (or extend the allowlist\n"
           "in the lint-testattr recipe with a stated reason).",
           file=sys.stderr,
       )
       sys.exit(1)

# Full CI pipeline (lint + test + docs)
ci: lint test
    cargo test --workspace --doc --exclude llm
    cargo doc --workspace --no-deps

# Build and open documentation
docs:
    cargo doc --workspace --no-deps --open

# Generate lcov coverage data (coverage.lcov).
coverage:
    cargo llvm-cov --lcov --output-path coverage.lcov

# Generate and open an HTML coverage report from coverage.lcov.
coverage-report:
    cargo llvm-cov report --html

# Run debtmap analysis and write debt.json.
debt: coverage
    debtmap analyze . --lcov coverage.lcov

# Apply the project license header to source files missing it.
apply-license:
   #!/bin/bash

   # --- CONFIGURATION ---
   NAME="{{COPYRIGHT_NAME}}"
   YEAR="{{COPYRIGHT_YEAR}}"
   # Add the extensions you want to target (space separated)
   EXTENSIONS=("rs")

   # The Header Template
   HEADER="Copyright (C) $YEAR $NAME

   This program is free software: you can redistribute it and/or modify
   it under the terms of the GNU Affero General Public License as
   published by the Free Software Foundation, either version 3 of the
   License, or (at your option) any later version.

   This program is distributed in the hope that it will be useful,
   but WITHOUT ANY WARRANTY; without even the implied warranty of
   MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
   GNU Affero General Public License for more details.

   You should have received a copy of the GNU Affero General Public License
   along with this program.  If not, see <https://www.gnu.org/licenses/>."

   # Convert header to a commented block (using // for JS/CPP style)
   # If you use Python only, change '// ' to '# ' below.
   COMMENTED_HEADER=$(echo "$HEADER" | sed 's/^/\/\/ /')

   # --- EXECUTION ---
   for ext in "${EXTENSIONS[@]}"; do
       echo "Processing .$ext files..."

       # Find files with the extension, excluding node_modules or hidden git folders
       find . -type f -name "*.$ext" -not -path "*/.*" -not -path "*node_modules*" | while read -r file; do

           # Check if "Copyright" already exists in the first 5 lines
           if head -n 5 "$file" | grep -iq "Copyright"; then
               echo "  Skipping $file (Header already exists)"
           else
               echo "  Adding header to $file"
               # Create a temporary file with header + original content
               { echo "$COMMENTED_HEADER"; echo ""; cat "$file"; } > "$file.tmp" && mv "$file.tmp" "$file"
           fi
       done
   done

   echo "Done!"

# Copy themes, personas, and prompts to user config directory
install-defaults:
    mkdir -p ~/.config/jinn/themes
    cp -r res/themes/*.toml ~/.config/jinn/themes/
    mkdir -p ~/.config/jinn/personas
    cp -r res/personas/*.md ~/.config/jinn/personas/
    mkdir -p ~/.config/jinn/prompts
    cp -r res/prompts/*.md ~/.config/jinn/prompts/
    mkdir -p ~/.config/jinn/workflows
    cp -r res/workflows/* ~/.config/jinn/workflows/
    mkdir -p ~/.agents/skills
    cp -r res/skills/* ~/.agents/skills/
    @echo "Themes installed to ~/.config/jinn/themes/"
    @echo "Personas installed to ~/.config/jinn/personas/"
    @echo "Prompts installed to ~/.config/jinn/prompts/"
    @echo "Workflows installed to ~/.config/jinn/workflows/"
    @echo "Skills installed to ~/.agents/skills/"

# Push main (and its annotated tags) to GitHub. git-primary: origin IS the
# GitHub repo the old fossil→git mirror used to populate, so syncing is a push.
sync-github:
   #!/bin/bash
   set -euo pipefail

   GITHUB_REMOTE="git@github.com:jayson-lennon/jinn.git"

   # Invariant check: this recipe's contract is "publish to the jinn GitHub
   # repo". If origin has been repointed, fail loudly instead of pushing
   # somewhere unexpected under muscle memory.
   ACTUAL=$(git remote get-url --push origin)
   if [ "$ACTUAL" != "$GITHUB_REMOTE" ]; then
       echo "Error: origin points at '$ACTUAL', expected '$GITHUB_REMOTE'" >&2
       exit 1
   fi

   echo '==> Pushing main and tags to GitHub'
   # --follow-tags pushes annotated tags reachable from main (what `just bump`
   # creates) without sweeping in every historical tag the way --tags would.
   git push origin main --follow-tags

# ---------------------------------------------------------------------------
# ---------------------------------------------------------------------------
# GitHub PR intake (patch-based; the landed commit is attributed to the
# contributor via its git author).
#
#   just gh-pr-list [STATE]    browse PRs (default open)
#   just gh-pr-view N          PR metadata in the terminal
#   just gh-pr-fetch N         download patch + metadata, extract the author
#   just gh-pr-apply N         apply the patch; conflicts become conflict markers
#   ...review, `just check`, `just test`...
#   just gh-pr-land N          commit, attributed to the contributor
#   just sync-github           push the landed commit to GitHub
#   just gh-pr-close N [MSG]   close the PR on GitHub with a note
#
# Per-PR artifacts live in target/gh-pr/N/ (ignored via the `target` glob).
# ---------------------------------------------------------------------------
export GH_REPO := "jayson-lennon/jinn"
GH_PR_STATE_DIR := "target/gh-pr"

# List GitHub PRs for the mirror (default: open).
gh-pr-list STATE="open":
    gh pr list --state {{STATE}} --limit 30 \
        --json number,title,author,headRefName \
        --jq '.[] | "#\(.number)  \(.author.login)  [\(.headRefName)]  \(.title)"'

# Show a GitHub PR's title, body, and branch in the terminal.
gh-pr-view N:
    gh pr view {{N}} \
        --json number,title,url,author,headRefName,headRefOid,additions,deletions,changedFiles \
        --jq '"#\(.number) \(.title)", "  by \(.author.login)  \(.headRefName)@\(.headRefOid[0:10])", "  +\(.additions) -\(.deletions) across \(.changedFiles) file(s)", "  \(.url)"'

# Fetch PR patch + extract contributor identity from the first commit's "From:" header
gh-pr-fetch N:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="{{GH_PR_STATE_DIR}}/{{N}}"
    mkdir -p "$dir"

    echo '==> Fetching PR metadata'
    gh pr view {{N}} \
        --json number,title,url,author,headRefName,headRepositoryOwner,isCrossRepository \
        > "$dir/meta.json"

    echo '==> Fetching patch'
    # NOTE: `gh pr diff` can exit 0 while writing an error string into the
    # file on some failure modes (observed: HTTP 422 → empty file). Guard
    # with a non-empty check too, not just the exit code.
    gh pr diff {{N}} --patch > "$dir/pr.patch"
    if [ ! -s "$dir/pr.patch" ] || ! grep -q '^From ' "$dir/pr.patch"; then
        echo "ERROR: no patch content for PR {{N}} (diff unavailable?)" >&2
        exit 1
    fi

    commits=$(grep -c '^From ' "$dir/pr.patch" || true)
    ident=$(awk 'NR>1 && /^From: */{ sub(/^From: */,""); print; exit }' "$dir/pr.patch")
    if [ -z "$ident" ]; then
        echo 'ERROR: no "From:" header in patch; cannot extract author identity.' >&2
        exit 1
    fi
    printf '%s\n' "$ident" > "$dir/identity"

    echo "    commits : $commits"
    echo "    author  : $ident"
    if [ "$commits" -gt 1 ]; then
        echo '    note    : multi-commit PR; identity taken from the FIRST commit' >&2
    fi
    echo "==> Saved: $dir/{meta.json,pr.patch,identity}"

# Apply a fetched PR patch to the working tree (refuses a dirty tree). Conflicting
# hunks land as inline conflict markers — resolve them by hand or hand them to an
# agent (they're greppable), then land with `just gh-pr-land`.
[doc("Apply a fetched PR patch to the working tree (refuses a dirty tree).")]
gh-pr-apply N:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="{{GH_PR_STATE_DIR}}/{{N}}"
    [ -f "$dir/pr.patch" ] || {
        echo "ERROR: no fetched patch for PR {{N}}; run: just gh-pr-fetch {{N}}" >&2
        exit 1
    }
    if [ -n "$(git status --porcelain)" ]; then
        echo 'ERROR: working tree has uncommitted changes; commit or revert first.' >&2
        exit 1
    fi
    branch=$(git branch --show-current)
    if [ "$branch" != "main" ]; then
        echo "Note: applying on branch '$branch', not main" >&2
    fi

    echo '==> Probing with dry run'
    if patch -p1 --dry-run < "$dir/pr.patch" >/dev/null 2>&1; then
        echo '==> Clean apply'
        patch -p1 < "$dir/pr.patch"
    else
        echo '==> Conflicts detected: writing conflict markers into the tree'
        patch -p1 --merge < "$dir/pr.patch" || true
        # drop patch's .orig backups; the conflicted files themselves carry
        # the pre-image in the `<<<<<<<` block, so .orig is redundant noise
        find . -name '*.orig' -not -path './.git/*' -not -path './target/*' -delete
        echo
        echo '    Conflicted spots (resolve every one, then delete its markers):'
        grep -rn '^<<<<<<<' --exclude-dir=.git --exclude-dir=target . || true
        echo
        echo '    After resolving:'
        echo '      just check && just test'
        echo "      just gh-pr-land {{N}}"
    fi
    echo "==> Applied. Then: just gh-pr-land {{N}}"

# Commit the applied patch attributed to the contributor (git author identity
# extracted from the patch by gh-pr-fetch).
# Refuses to run while unresolved conflict markers remain in the tree.
[doc("Commit an applied PR patch attributed to its contributor, refusing on unresolved conflict markers.")]
gh-pr-land N:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="{{GH_PR_STATE_DIR}}/{{N}}"
    [ -f "$dir/pr.patch" ] || {
        echo "ERROR: no fetched patch for PR {{N}}; run: just gh-pr-fetch {{N}}" >&2
        exit 1
    }
    if [ -z "$(git status --porcelain)" ]; then
        echo 'ERROR: nothing to commit; apply the patch first: just gh-pr-apply {{N}}' >&2
        exit 1
    fi

    if grep -rn '^<<<<<<<' --exclude-dir=.git --exclude-dir=target . >/dev/null 2>&1; then
        echo 'ERROR: unresolved conflict markers remain:' >&2
        grep -rn '^<<<<<<<' --exclude-dir=.git --exclude-dir=target . | head -10 >&2
        echo 'Resolve them (or hand them to an agent), then re-run.' >&2
        exit 1
    fi

    ident=$(<"$dir/identity")
    url=$(jq -r '.url' "$dir/meta.json")

    # The contributor is recorded as the commit's git author; the committer
    # stays the local user, so the landing is attributable without any
    # account provisioning.
    msg=$(printf "Apply PR #{{N}} from %s\n\nPicked from %s\n" \
        "$ident" "$url")
    git add -A
    git commit -q --author="$ident" -m "$msg"

    hash=$(git rev-parse --short HEAD)
    echo
    echo "==> Landed as $hash (author: $ident)"
    echo '    next: just check && just test   (if not already done)'
    echo '    then: just sync-github'
    echo "    then: just gh-pr-close {{N}} 'Landed in $hash; pushed shortly.'"

# Opt-in: close the PR on GitHub after landing
gh-pr-close N COMMENT="":
    #!/usr/bin/env bash
    set -euo pipefail
    args=()
    if [ -n "{{COMMENT}}" ]; then
        args+=(--comment "{{COMMENT}}")
    fi
    gh pr close {{N}} "${args[@]}"

# Bump version (major/minor/patch), commit, and tag in git
bump LEVEL:
    #!/usr/bin/env bash
    set -euo pipefail

    # --- Validate input ---
    case "{{LEVEL}}" in
        major|minor|patch) ;;
        *) echo "Usage: just bump <major|minor|patch>" >&2; exit 1 ;;
    esac

    # --- Pre-flight: must be on main ---
    BRANCH=$(git branch --show-current)
    if [ "$BRANCH" != "main" ]; then
        echo "Error: must be on main (currently on '$BRANCH')" >&2
        exit 1
    fi

    # --- Pre-flight: working tree must be clean ---
    if [ -n "$(git status --porcelain)" ]; then
        echo "Error: working tree has uncommitted changes" >&2
        exit 1
    fi

    # --- Compute new version ---
    CURRENT=$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version = "\(.*\)"/\1/p}' Cargo.toml)
    NEW=$(rust-script scripts/bump-version.rs "$CURRENT" "{{LEVEL}}")

    # --- Resolve tag conflicts ---
    CANDIDATE="$NEW"
    ATTEMPTS=0
    while git rev-parse -q --verify "refs/tags/v${CANDIDATE}" >/dev/null; do
        echo "Tag v${CANDIDATE} already exists, skipping..."
        ATTEMPTS=$((ATTEMPTS + 1))
        if [ "$ATTEMPTS" -ge 100 ]; then
            echo "Error: too many tag collisions, aborting" >&2
            exit 1
        fi
        CANDIDATE=$(rust-script scripts/bump-version.rs "$CANDIDATE" "patch")
    done

    # --- Update files ---
    sed -i "/^\[workspace\.package\]/,/^\[/{s/^version = \".*\"/version = \"$CANDIDATE\"/}" Cargo.toml

    # --- Regenerate Cargo.lock for the new workspace version ---
    cargo update --workspace

    # --- Commit ---
    git add -A && git commit -q -m "Bump version to ${CANDIDATE}"

    # --- Tag the current commit (annotated) ---
    git tag -a -m "v${CANDIDATE}" "v${CANDIDATE}"

    echo "Bumped to ${CANDIDATE}, committed and tagged as v${CANDIDATE}"


# Symlink ./target to /dev/shm/<parent-dir> for faster builds
shm-target:
    #!/bin/bash
    set -euo pipefail

    NAME="$(basename '{{justfile_directory()}}')"
    SHM_DIR="/dev/shm/${NAME}"

    if [ -L target ]; then
        CURRENT="$(readlink target)"
        if [ "$CURRENT" = "$SHM_DIR" ]; then
            echo "target already points to $SHM_DIR"
            exit 0
        fi
        echo "target is a symlink to $CURRENT — removing"
        rm target
    fi

    if [ -d target ]; then
        echo "target/ exists as a directory — removing"
        rm -rf target
    fi

    mkdir -p "$SHM_DIR"
    ln -s "$SHM_DIR" target
    echo "target -> $SHM_DIR"

# Remove the /dev/shm/<parent-dir> symlink and directory
unshm-target:
    #!/bin/bash
    set -euo pipefail

    NAME="$(basename '{{justfile_directory()}}')"
    SHM_DIR="/dev/shm/${NAME}"

    if [ -L target ]; then
        CURRENT="$(readlink target)"
        if [ "$CURRENT" = "$SHM_DIR" ]; then
            rm target
            echo "removed target symlink"
        fi
    fi

    if [ -d "$SHM_DIR" ]; then
        rm -rf "$SHM_DIR"
        echo "removed $SHM_DIR"
    else
        echo "$SHM_DIR does not exist"
    fi

# Build the Arch package in ./build (isolated src/pkg scratch dirs; run from repo root)
pkg:
    @mkdir -p build
    @BUILDDIR="$(pwd)/build" PKGDEST="$(pwd)/build" makepkg -f

# --- Release (cargo-binstall) -----------------------------------------------
#
# `jinn` is distributed as a prebuilt binary attached to GitHub releases.
# `cargo-binstall` downloads it (see [package.metadata.binstall] in Cargo.toml).
# Source tarballs are auto-generated by GitHub per tag — no work needed.
#
# Prerequisite: install the GitHub CLI and authenticate once:
#     https://cli.github.com/   then   gh auth login

# Cross-build the release tarball for TARGET. Requires cargo-xwin:
# run `just setup-windows-cross` once before first use.
[doc("Cross-build the release tarball for TARGET.")]
build-release-tarball TARGET="x86_64-unknown-linux-gnu":
    #!/usr/bin/env bash
    set -euo pipefail

    VERSION=$(sed -n '/^\[workspace\.package\]/,/^[\[]/{s/^version = "\(.*\)"/\1/p}' Cargo.toml)
    TARGET="{{TARGET}}"
    STAGE="jinn-${TARGET}-v${VERSION}"
    TARBALL="${STAGE}.tgz"

    # --- Prerequisites for cross targets ---
    case "${TARGET}" in
        x86_64-pc-windows-msvc)
            if ! command -v cargo-xwin >/dev/null 2>&1; then
                echo "Error: cargo-xwin is not installed." >&2
                echo "  Run: just setup-windows-cross" >&2
                exit 1
            fi
            ;;
    esac
    if ! rustup target list --installed 2>/dev/null | grep -qx "${TARGET}"; then
        echo "Error: rustup target '${TARGET}' is not installed." >&2
        echo "  Run: rustup target add ${TARGET}" >&2
        exit 1
    fi

    # Binary name: cargo emits foo.exe for windows targets, foo otherwise.
    BIN="jinn"
    case "${TARGET}" in
        *windows*) BIN="jinn.exe" ;;
    esac

    echo "==> Building release binary (target ${TARGET})"
    case "${TARGET}" in
        x86_64-pc-windows-msvc)
            # Cross-build via cargo-xwin (clang-cl for C, lld-link for link,
            # MSVC CRT + Windows SDK fetched by xwin). Scoped to this process:
            # no [target.*] config is checked in, so native Windows builders
            # are unaffected. +crt-static keeps the binary self-contained
            # (no VC runtime needed).
            #
            # /ignore:4099 tells lld-link to drop the LNK4099 "Cannot use
            # debug info" warning at the source, rather than suppressing the
            # whole class of linker diagnostics via `-A linker_messages`. The
            # static CRT .lib files reference Microsoft-internal debug-info
            # PDBs (D:\a\_work\1\s\binaries\...) that were never
            # redistributable, so a cross link always emits ~60 of these.
            # Only debug info is dropped — the linked binary is unaffected.
            # Narrowing to one code keeps genuine linker warnings (LNK4217,
            # LNK4199, ...) visible; `release` validates the artifact
            # structurally regardless.
            export XWIN_ARCH=x86_64
            export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=lld-link
            export RUSTFLAGS="-Ctarget-feature=+crt-static -Clink-arg=/ignore:4099"
            cargo xwin build --release --target "${TARGET}"
            ;;
        *)
            cargo build --release --target "${TARGET}"
            ;;
    esac

    echo "==> Packaging ${TARBALL}"
    STAGE_DIR="$(mktemp -d)"
    mkdir -p "${STAGE_DIR}/${STAGE}"
    cp "target/${TARGET}/release/${BIN}" "${STAGE_DIR}/${STAGE}/${BIN}"
    tar -czf "${TARBALL}" -C "${STAGE_DIR}" "${STAGE}"
    rm -rf "${STAGE_DIR}"

    echo "==> Created ${TARBALL}"

# One-time setup for cross-building the Windows release tarball from Linux
# (idempotent; safe to re-run). Installs the rustup target, the cargo-xwin
# driver (clang-cl for C, lld-link for link), and prefetches the MSVC CRT +
# Windows SDK subsets xwin needs.
[doc("One-time setup for cross-building Windows release tarballs from Linux.")]
setup-windows-cross:
    #!/usr/bin/env bash
    set -euo pipefail

    fail() {
        echo "Error: $1" >&2
        echo "  $2" >&2
        exit 1
    }

    TARGET="x86_64-pc-windows-msvc"

    # --- Prerequisites: cargo/rustup + LLVM binutils (clang-cl, lld-link) ---
    command -v rustup >/dev/null 2>&1 || fail "rustup is not installed." "Install from https://rustup.rs"
    command -v cargo >/dev/null 2>&1 || fail "cargo is not installed." "Install from https://rustup.rs"
    command -v clang-cl >/dev/null 2>&1 \
        || fail "clang-cl is not on PATH." \
           "  Arch: pacman -S clang lld   Debian/Ubuntu: apt install clang lld"
    command -v lld-link >/dev/null 2>&1 \
        || fail "lld-link is not on PATH." \
           "  Arch: pacman -S clang lld   Debian/Ubuntu: apt install clang lld"

    # --- Install the rustup target ---
    rustup target add "${TARGET}"

    # --- Install cargo-xwin ---
    cargo install cargo-xwin --locked

    # --- Prefetch the MSVC CRT + Windows SDK subsets and validate the toolchain ---
    echo '==> Prefetching MSVC CRT/SDK via cargo-xwin (a few hundred MB, cached locally)...'
    PREFETCH_DIR="$(mktemp -d)"
    cargo init --name xwin_prefetch --vcs none "${PREFETCH_DIR}"
    (
        cd "${PREFETCH_DIR}" \
            && XWIN_ARCH=x86_64 cargo xwin build --release --target "${TARGET}"
    )
    rm -rf "${PREFETCH_DIR}"
    echo "==> Windows cross toolchain ready (target ${TARGET})"

# Release TAG to GitHub + smoke-test cargo-binstall (after `just bump`)
release TAG:
    #!/usr/bin/env bash
    set -euo pipefail

    REPO="jayson-lennon/jinn"
    LINUX_TARGET="x86_64-unknown-linux-gnu"
    WINDOWS_TARGET="x86_64-pc-windows-msvc"
    VERSION=$(sed -n '/^\[workspace\.package\]/,/^[\[]/{s/^version = "\(.*\)"/\1/p}' Cargo.toml)
    if [ "{{TAG}}" != "v${VERSION}" ]; then
        echo "Error: tag '{{TAG}}' does not match Cargo.toml version 'v${VERSION}'." >&2
        echo "  Run 'just bump <major|minor|patch>' first, then 'just release v${VERSION}'." >&2
        exit 1
    fi

    # --- Pre-flight: gh must be installed ---
    if ! command -v gh >/dev/null 2>&1; then
        echo "Error: GitHub CLI (gh) is not installed." >&2
        echo "  Install from https://cli.github.com/ then run: gh auth login" >&2
        exit 1
    fi

    # --- Pre-flight: gh must be authenticated ---
    if ! gh auth status >/dev/null 2>&1; then
        echo "Error: gh is not authenticated." >&2
        echo "  Run: gh auth login" >&2
        exit 1
    fi

    # --- 1. Push main (and tags) to GitHub ---
    just sync-github

    # --- 2. Build the cargo-binstall tarballs (linux + windows) ---
    just build-release-tarball "${LINUX_TARGET}"
    just build-release-tarball "${WINDOWS_TARGET}"

    TARBALL_LINUX="jinn-x86_64-unknown-linux-gnu-v${VERSION}.tgz"
    TARBALL_WINDOWS="jinn-x86_64-pc-windows-msvc-v${VERSION}.tgz"

    # --- 3. Verify the built artifacts locally, BEFORE publishing anything ---
    # Every check below must pass before step 4 touches the GitHub release, so
    # a bad tarball or binary aborts the recipe rather than going live. The
    # recipe's `set -euo pipefail` makes the first failing check exit 1, which
    # leaves the release untouched.
    #
    # The [package.metadata.binstall] templates resolve
    # jinn-<target>-v<version>.tgz -> jinn-<target>-v<version>/<bin><binary-ext>,
    # so the tarball member path IS what binstall looks up. Checking the
    # members (and running each binary) verifies the template against the
    # assets we just built — no network round-trip needed.
    echo '==> Verifying tarball layouts (binstall template: {name}-{target}-v{version}/{bin}{binary-ext})'
    [ "$(tar -tzf "${TARBALL_LINUX}" | grep -cx "jinn-${LINUX_TARGET}-v${VERSION}/jinn")" -eq 1 ] \
        || { echo "Error: ${TARBALL_LINUX} does not contain jinn-${LINUX_TARGET}-v${VERSION}/jinn" >&2; exit 1; }
    [ "$(tar -tzf "${TARBALL_WINDOWS}" | grep -cx "jinn-${WINDOWS_TARGET}-v${VERSION}/jinn.exe")" -eq 1 ] \
        || { echo "Error: ${TARBALL_WINDOWS} does not contain jinn-${WINDOWS_TARGET}-v${VERSION}/jinn.exe" >&2; exit 1; }

    echo "==> Linux binary reports: $(./target/${LINUX_TARGET}/release/jinn --version)"

    # Windows artifact checks: PE32+ x86-64 structure (msvc linker output)
    # and a static CRT — the binary must import no vcruntime*/ucrtbase DLLs
    # (kernel32 et al. are expected; a DLL CRT means +crt-static was dropped).
    JINN_EXE="./target/${WINDOWS_TARGET}/release/jinn.exe"
    # file(1) word order varies across libmagic versions ("x86-64, for MS
    # Windows" vs "for MS Windows ..., x86-64"); accept both.
    file "${JINN_EXE}" | grep -Eq 'PE32\+ executable.*(x86-64.*MS Windows|MS Windows.*x86-64)' \
        || { echo "Error: ${JINN_EXE} is not a PE32+ x86-64 Windows executable:" >&2; file "${JINN_EXE}" >&2; exit 1; }
    echo "==> Windows binary structure: $(file -b "${JINN_EXE}")"
    if command -v objdump >/dev/null 2>&1; then
        if objdump -x "${JINN_EXE}" 2>/dev/null | grep -Eiq 'vcruntime|ucrtbase'; then
            echo "Error: ${JINN_EXE} imports a VC runtime DLL — static CRT (+crt-static) was dropped." >&2
            exit 1
        fi
        echo '==> Windows binary CRT: static (no vcruntime/ucrtbase imports)'
    else
        echo '==> objdump not on PATH; skipping static-CRT import scan'
    fi

    if command -v wine >/dev/null 2>&1; then
        echo "==> Windows binary reports: $(WINEDEBUG=-all wine "${JINN_EXE}" --version 2>/dev/null)"
    else
        echo '==> wine not on PATH; skipping windows binary run'
    fi

    # --- 4. Create the release if it doesn't exist, else upload ---
    # Reached only when every check in step 3 passed.
    if gh release view "{{TAG}}" --repo "${REPO}" >/dev/null 2>&1; then
        echo "==> Uploading tarballs to existing release {{TAG}}"
        gh release upload "{{TAG}}" "${TARBALL_LINUX}" "${TARBALL_WINDOWS}" --repo "${REPO}" --clobber
    else
        echo "==> Creating release {{TAG}} and uploading tarballs"
        gh release create "{{TAG}}" "${TARBALL_LINUX}" "${TARBALL_WINDOWS}" --repo "${REPO}" --generate-notes
    fi

    echo "==> Done. https://github.com/${REPO}/releases/tag/{{TAG}}"
