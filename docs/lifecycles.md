# Session Lifecycles

Session lifecycles let you bootstrap (setup) and tear down working environments
when creating or closing a chat session. They bridge the gap between your tools
(e.g., version control branches, build scripts) and your chat session's working
directory.

## How It Works

1. You define lifecycle recipes in `~/.config/nullslop/nullslop.toml`.
2. When creating a new session, you pick a lifecycle from the picker.
3. The **setup command** runs asynchronously — its last non-empty stdout line
   becomes the session's [CWD](./cwd.md).
4. When closing the session, the **teardown command** runs with the same arguments.
5. The implicit **"blank"** lifecycle (no setup/teardown commands) is always
   available and is the default when no lifecycle is selected.

```
Session lifecycle picker
  blank                    — No setup
  fossil branch *          — Open a fossil branch in a new worktree
  git feature branch *     — Create a feature branch and checkout
```

Lifecycles with parameters (`*`) show an **arg input popup** where you enter
the positional arguments before the session is created.

## Configuration Format

Lifecycles are defined in `~/.config/nullslop/nullslop.toml` under `[[session_lifecycle]]`
table arrays:

```toml
last_model = "anthropic/claude-sonnet-4-20250514"

[[session_lifecycle]]
name = "fossil branch"
description = "Open a fossil branch in a new worktree"
setup_command = "~/.config/nullslop/scripts/fossil-branch.sh $1"
teardown_command = "~/.config/nullslop/scripts/fossil-cleanup.sh $1"
```

### Fields

| Field             | Required | Description |
|-------------------|----------|-------------|
| `name`            | Yes      | Displayed in the lifecycle picker |
| `description`     | No       | Shown below the name in the picker |
| `setup_command`   | No       | Shell command run on session creation. Last stdout line → CWD. Omit for "blank" lifecycle |
| `teardown_command`| No       | Shell command run on session close. Same args as setup |

### Parameter Syntax

Parameters are filled from user input when creating the session:

| Syntax  | Example              | Description                |
|---------|----------------------|----------------------------|
| `$1`..`$9` | `script.sh $1`   | Positional parameter       |
| `<name>`    | `script.sh <branch>` | Named parameter (same as positional) |
| `$@`, `$*`  | `script.sh $@`   | Splat — consumes all remaining args |

Examples:

```toml
# $1 and $2 positional parameters
setup_command = "scripts/setup.sh $1 $2"

# Named parameters (equivalent to positional)
setup_command = "scripts/setup.sh <branch> <target>"

# Splat — accepts any number of args
setup_command = "scripts/run.sh $@"

# Mixed — first two args are positional, rest are splatted
setup_command = "scripts/deploy.sh $1 $2 $@"
```

Parameters are deduplicated — if `$1` appears twice in the command, the user
only provides it once.

---

## Example Lifecycles

### 1. Fossil Branch — Create a branch in a new worktree

Creates a fossil branch in a fresh worktree (one branch per session).

**`~/.config/nullslop/nullslop.toml`:**

```toml
[[session_lifecycle]]
name = "fossil branch"
description = "Create a fossil branch in a new worktree"
setup_command = "~/.config/nullslop/scripts/fossil-branch.sh $1"
teardown_command = "~/.config/nullslop/scripts/fossil-cleanup.sh $1"
```

**`~/.config/nullslop/scripts/fossil-branch.sh`:**

```bash
#!/bin/bash
set -euo pipefail

BRANCH="$1"
WORKTREE="/tmp/nullslop-worktrees/$BRANCH"

# Create a fresh checkout directory
mkdir -p "$WORKTREE"

# Navigate to the fossil repository root
FOSSIL_REPO="/mnt/zed/repos/nullslop/nullslop.fossil"
cd /tmp

# Open the repository at the worktree path
fossil open "$FOSSIL_REPO" --worktree "$WORKTREE"

# Create and switch to the new branch
fossil branch new "$BRANCH" trunk

# Output the worktree path — this becomes the session CWD
echo "$WORKTREE"
```

**`~/.config/nullslop/scripts/fossil-cleanup.sh`:**

```bash
#!/bin/bash
set -euo pipefail

BRANCH="$1"
WORKTREE="/tmp/nullslop-worktrees/$BRANCH"

# Close the fossil checkout
cd "$WORKTREE"
fossil close --force

# Remove the worktree
rm -rf "$WORKTREE"
```

**Usage flow:**
1. Open lifecycle picker → select "fossil branch"
2. Enter branch name → `fix-bug-123`
3. Session opens with CWD = `/tmp/nullslop-worktrees/fix-bug-123`
4. All tool calls (bash, read, write) execute relative to that directory
5. On session close: branch is closed and worktree is cleaned up

---

### 2. Git Feature Branch — Create a feature branch

Creates a git feature branch from trunk and checks it out.

```toml
[[session_lifecycle]]
name = "git feature branch"
description = "Create a feature branch and checkout"
setup_command = "~/.config/nullslop/scripts/git-setup.sh <project> <branch>"
teardown_command = "~/.config/nullslop/scripts/git-cleanup.sh <project> <branch>"
```

**`~/.config/nullslop/scripts/git-setup.sh`:**

```bash
#!/bin/bash
set -euo pipefail

PROJECT="$1"
BRANCH="$2"
REPO_DIR="$HOME/projects/$PROJECT"

cd "$REPO_DIR"
git checkout trunk
git pull origin trunk
git checkout -b "$BRANCH"
echo "$REPO_DIR"
```

**`~/.config/nullslop/scripts/git-cleanup.sh`:**

```bash
#!/bin/bash
set -euo pipefail

PROJECT="$1"
BRANCH="$2"
REPO_DIR="$HOME/projects/$PROJECT"

cd "$REPO_DIR"
git checkout trunk
git branch -D "$BRANCH" 2>/dev/null || true
```

**Usage flow:**
1. Pick "git feature branch" from the lifecycle picker
2. Enter: `nullslop fix-readme-typo`
3. Session opens at `~/projects/nullslop/` with the `fix-readme-typo` branch checked out

---

### 3. Temporary Scratch Directory — Isolated sandbox

Creates a temporary directory for each session with zero cleanup concerns.

```toml
[[session_lifecycle]]
name = "scratch dir"
description = "Create a temp directory for the session"
setup_command = "mktemp -d /tmp/nullslop-scratch-XXXXXXX"
teardown_command = "rm -rf $1"
```

Note the implicit `$1`: the setup command's output (the temp dir path) is stored
as the session's lifecycle args, and the teardown command uses `$1` to reference it.

**Usage flow:**
1. Pick "scratch dir" from the lifecycle picker
2. No args needed (the lifecycle has no explicit params)
3. Session opens at a newly created temp directory like `/tmp/nullslop-scratch-aB3xY2z`
4. On close: the directory is automatically deleted

---

### 4. Custom Project with Build Step

Clones or pulls a repo, runs a build, and sets up environment variables.

```toml
[[session_lifecycle]]
name = "project setup"
description = "Clone/pull and build a project"
setup_command = "~/.config/nullslop/scripts/setup-project.sh $1"
teardown_command = "~/.config/nullslop/scripts/cleanup-project.sh $1"
```

**`~/.config/nullslop/scripts/setup-project.sh`:**

```bash
#!/bin/bash
set -euo pipefail

PROJECT="$1"
TARGET="$HOME/worktrees/$PROJECT"

# Clone if missing, pull if existing
if [ ! -d "$TARGET" ]; then
    git clone "git@github.com:my-org/$PROJECT.git" "$TARGET"
else
    cd "$TARGET"
    git pull origin main
fi

# Build
cd "$TARGET"
cargo build 2>&1

# Output the working directory — becomes the CWD
echo "$TARGET"
```

**Usage flow:**
1. Pick "project setup"
2. Enter project name → `nullslop`
3. Session opens at `~/worktrees/nullslop/` with the latest code built
4. If the build fails, the command errors and an error message appears in the
   session history (the session is still created but CWD falls back to default)

---

### 5. Docker Container Work Environment

Spins up a Docker container for an isolated development environment.

```toml
[[session_lifecycle]]
name = "docker workspace"
description = "Spin up a Docker container for development"
setup_command = "~/.config/nullslop/scripts/docker-setup.sh $1"
teardown_command = "~/.config/nullslop/scripts/docker-teardown.sh $1"
```

**`~/.config/nullslop/scripts/docker-setup.sh`:**

```bash
#!/bin/bash
set -euo pipefail

IMAGE="$1"
CONTAINER_NAME="nullslop-$(date +%s)-$$"

docker run -d \
    --name "$CONTAINER_NAME" \
    -v "$HOME/projects:/projects" \
    "$IMAGE" \
    sleep infinity

# Get the working directory inside the container
WORKDIR=$(docker exec "$CONTAINER_NAME" pwd)

# Save the container name for teardown (must be accessible later)
echo "$CONTAINER_NAME" > /tmp/nullslop-container-$CONTAINER_NAME.txt

echo "$WORKDIR"
```

**`~/.config/nullslop/scripts/docker-teardown.sh`:**

```bash
#!/bin/bash
set -euo pipefail

IMAGE="$1"
CONTAINER_FILE="/tmp/nullslop-container-*.txt"
CONTAINER=$(cat $CONTAINER_FILE 2>/dev/null || echo "")

if [ -n "$CONTAINER" ]; then
    docker stop "$CONTAINER" 2>/dev/null || true
    docker rm "$CONTAINER" 2>/dev/null || true
    rm -f $CONTAINER_FILE
fi
```

---

### 6. Multi-Argument: Test Runner

Runs tests from a specific directory with configurable flags.

```toml
[[session_lifecycle]]
name = "test runner"
description = "Start a session in a test directory with flags"
setup_command = "~/.config/nullslop/scripts/test-setup.sh <suite> <flags>"
```

**Usage flow:**
1. Pick "test runner"
2. Enter: `integration --verbose`
3. The setup script receives two args: `suite=integration`, `flags=--verbose`

The absence of `teardown_command` means nothing happens when the session closes —
this lifecycle is setup-only.

---

### 7. Blank — No Setup (Implicit)

The "blank" lifecycle is the default when no lifecycle is selected. It creates a
session with no setup command and no teardown. The session CWD is set to the
global default CWD (the directory where nullslop was launched).

You don't need to define this in `nullslop.toml` — it's always available.

## Error Handling

| Scenario | Behavior |
|----------|----------|
| Setup command fails (non-zero exit) | Error message appended to session history. CWD falls back to global default |
| Setup command produces no output | `NoOutput` error appended to session history. CWD falls back to global default |
| Teardown fails (non-zero exit) | Warning logged via `tracing`. Session is still removed |
| User provides too few args | Arg input popup stays open — validation rejects insufficient args |
| Too many args (no splat) | Extra args are silently ignored |

## Advanced: Same Args for Setup and Teardown

The teardown command receives **the same args** as the setup command. This lets
the teardown know which branch, project, or container to clean up without needing
to persist state externally.

```toml
[[session_lifecycle]]
name = "fossil branch"
setup_command = "fossil-branch.sh $1"
teardown_command = "fossil-cleanup.sh $1"
```

Both `fossil-branch.sh` and `fossil-cleanup.sh` receive `$1` = the same branch name.
