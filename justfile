COPYRIGHT_NAME := "Jayson Lennon"
COPYRIGHT_YEAR := "2026"

export RSTEST_TIMEOUT := "10"

fossil-branch NAME:
    fossil commit -m "Open {{NAME}}" --branch {{NAME}} --allow-empty

# Commit changes: stage all adds/removes (with `--dotfiles`) and commit.
commit MSG:
    fossil addremove --dotfiles && fossil commit -m "{{MSG}}"

test:
    cargo test --workspace

check:
    cargo check --workspace

# Build the in-repo wasm plugins from source (needs the wasm32-wasip2
# target). Plugins are delivered as source and compiled per machine — then
# installed like any user plugin:
#   jinn plugin install target/wasm32-wasip2/release/<crate>.wasm \
#     --grant '<config_dir>/themes'   # themes plugin scans granted dirs
# Equivalent to `jinn plugin build plugins/<dir>` per plugin.
build-plugins:
    cargo build -p jinn-plugin-themes --target wasm32-wasip2 --release


clippy:
    cargo clippy --workspace --all-targets

fmt:
    cargo fmt -- --check

fmt-fix:
    cargo fmt

# Run all linters (check + clippy + fmt check)
lint:
    cargo check --workspace
    just clippy
    cargo fmt -- --check

# Full CI pipeline (lint + test + docs)
ci: lint test
    cargo test --workspace --doc --exclude llm
    cargo doc --workspace --no-deps

# Run all cucumber tests
cucumber:
    cargo test --test e2e -p jinn-e2e

# Rebuild the dao compile-time validation database. Run this if `#[query]`
# validation acts stale after editing migrations in jinn-session-schema —
# it forces jinn-domain's build.rs to recreate the DB on the next check.
dao-db-rebuild:
    cargo clean -p jinn-domain


# Build and open documentation
docs:
    cargo doc --workspace --no-deps --open

coverage:
    cargo llvm-cov --lcov --output-path coverage.lcov

coverage-report:
    cargo llvm-cov report --html

debt: coverage
    debtmap analyze . --lcov coverage.lcov

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

# Check inline test modules for excessive length
lint-testlength:
   #!/usr/bin/env python3
   import os
   import re

   root = os.getcwd()
   max_lines = 200
   found = 0

   for dirpath, _, filenames in os.walk(root):
       # Skip vendor directory
       if 'target' in dirpath.split(os.sep):
           continue
       if 'vendor' in dirpath.split(os.sep):
           continue
       for fn in filenames:
           if not fn.endswith('.rs'):
               continue
           fpath = os.path.join(dirpath, fn)
           with open(fpath) as f:
               lines = f.readlines()
           for i, line in enumerate(lines):
               if line.strip() != '#[cfg(test)]':
                   continue
               # Look at next non-empty line
               j = i + 1
               while j < len(lines) and lines[j].strip() == '':
                   j += 1
               if j >= len(lines):
                   continue
               match = re.match(r'^mod\s+(\w+)\s*\{', lines[j].strip())
               if not match:
                   continue
               # Count lines by tracking brace depth
               depth = 0
               end_line = None
               for k in range(j, len(lines)):
                   for ch in lines[k]:
                       if ch == '{':
                           depth += 1
                       elif ch == '}':
                           depth -= 1
                   if depth == 0 and k >= j:
                       end_line = k
                       break
               if end_line is None:
                   continue
               mod_lines = end_line - i + 1
               if mod_lines > max_lines:
                   relpath = os.path.relpath(fpath, root)
                   print(f"WARN: {relpath}:{i + 1}: test module is {mod_lines} lines (max {max_lines})")
                   found += 1

   if found:
       print(f"\n{found} inline test module(s) exceed {max_lines} lines")

# Copy themes, personas, and prompts to user config directory
install-defaults:
    mkdir -p ~/.config/jinn/themes
    cp -r res/themes/*.toml ~/.config/jinn/themes/
    mkdir -p ~/.config/jinn/personas
    cp -r res/personas/*.md ~/.config/jinn/personas/
    mkdir -p ~/.config/jinn/prompts
    cp -r res/prompts/*.md ~/.config/jinn/prompts/
    mkdir -p ~/.agents/skills
    cp -r res/skills/* ~/.agents/skills/
    @echo "Themes installed to ~/.config/jinn/themes/"
    @echo "Personas installed to ~/.config/jinn/personas/"
    @echo "Prompts installed to ~/.config/jinn/prompts/"
    @echo "Skills installed to ~/.agents/skills/"

# Report stale Fossil locks (hung processes + stale journal files)
fossil-unlock:
    #!/usr/bin/env bash
    set -euo pipefail

    REPO="$(fossil status 2>/dev/null | sed -n 's/^repository: *//p')"
    if [ -z "$REPO" ]; then
        echo "Error: not inside a Fossil checkout" >&2; exit 1
    fi

    REPO="$(readlink -f "$REPO")"
    REPO_DIR="$(dirname "$REPO")"
    CHECKOUT_ROOT="$(pwd)"

    # Build the list of DB files worth checking: the repo, the current checkout,
    # and any sibling checkouts that live next to the .fossil file (e.g. ./pins).
    # Locks can live in ANY of these — not just where you're standing.
    TARGETS=("$REPO")
    while IFS= read -r -d '' f; do
        TARGETS+=("$f")
    done < <(find "$CHECKOUT_ROOT" "$REPO_DIR" -maxdepth 2 -name '.fslckout' -print0 2>/dev/null || true)
    # Dedupe (preserve order)
    SEEN=""; FILES=()
    for t in "${TARGETS[@]}"; do
        case "$SEEN" in
            *"|$t|"*) ;;
            *) SEEN="$SEEN|$t|"; FILES+=("$t") ;;
        esac
    done

    FOUND=0

    # 1. Hung Fossil processes holding any target DB
    echo '==> Checking for hung Fossil processes...'
    for db in "${FILES[@]}"; do
        PIDS=$(lsof "$db" 2>/dev/null | awk 'NR>1 && $1=="fossil" {print $2}' | sort -u) || true
        if [ -n "$PIDS" ]; then
            for pid in $PIDS; do
                CMD=$(ps -p "$pid" -o args= 2>/dev/null || echo "<exited>")
                echo "  stale PID $pid on $db: $CMD"
                FOUND=$((FOUND + 1))
            done
        fi
    done
    if [ "$FOUND" -eq 0 ]; then
        echo '  none found'
    fi

    # 2. Stale journal / WAL / SHM files next to each target
    echo '==> Checking for stale journal files...'
    JFOUND=0
    for db in "${FILES[@]}"; do
        d="$(dirname "$db")"
        while IFS= read -r -d '' f; do
            echo "  stale: $f"
            JFOUND=$((JFOUND + 1))
        done < <(find "$d" -maxdepth 1 \
            \( -name '.fslckout-journal' -o -name '.fslckout-wal' -o -name '.fslckout-shm' \
            -o -name '*.fossil-journal' -o -name '*.fossil-wal' -o -name '*.fossil-shm' \) -print0 2>/dev/null || true)
    done
    FOUND=$((FOUND + JFOUND))
    if [ "$JFOUND" -eq 0 ]; then
        echo '  none found'
    fi

    echo ""
    if [ "$FOUND" -gt 0 ]; then
        echo "Found $FOUND issue(s). Run 'just fossil-unlock-fix' to resolve."
    else
        echo "No lock issues found."
    fi

# Fix stale Fossil locks (kill hung processes + remove stale journal files)
fossil-unlock-fix:
    #!/usr/bin/env bash
    set -euo pipefail

    REPO="$(fossil status 2>/dev/null | sed -n 's/^repository: *//p')"
    if [ -z "$REPO" ]; then
        echo "Error: not inside a Fossil checkout" >&2; exit 1
    fi

    REPO="$(readlink -f "$REPO")"
    REPO_DIR="$(dirname "$REPO")"
    CHECKOUT_ROOT="$(pwd)"

    TARGETS=("$REPO")
    while IFS= read -r -d '' f; do
        TARGETS+=("$f")
    done < <(find "$CHECKOUT_ROOT" "$REPO_DIR" -maxdepth 2 -name '.fslckout' -print0 2>/dev/null || true)
    SEEN=""; FILES=()
    for t in "${TARGETS[@]}"; do
        case "$SEEN" in
            *"|$t|"*) ;;
            *) SEEN="$SEEN|$t|"; FILES+=("$t") ;;
        esac
    done

    FIXED=0
    ME="$(whoami)"

    # 1. Kill hung Fossil processes (SIGTERM, escalate to SIGKILL; force stopped procs)
    echo '==> Killing hung Fossil processes...'
    KILLED=0
    for db in "${FILES[@]}"; do
        PIDS=$(lsof "$db" 2>/dev/null | awk 'NR>1 && $1=="fossil" {print $2}' | sort -u) || true
        for pid in $PIDS; do
            OWNER=$(stat -c '%U' "/proc/$pid" 2>/dev/null || echo "")
            if [ "$OWNER" != "$ME" ]; then
                echo "  skipping PID $pid on $db (owned by $OWNER, not $ME)"
                continue
            fi
            if ! kill -0 "$pid" 2>/dev/null; then
                echo "  PID $pid on $db already gone"
                continue
            fi
            CMD=$(ps -p "$pid" -o args= 2>/dev/null || echo "<exited>")
            STATE=$(ps -p "$pid" -o stat= 2>/dev/null | cut -c1 || echo "?")
            if [ "$STATE" = "T" ]; then
                # Stopped/traced: SIGTERM can't be delivered. Force.
                kill -9 "$pid" 2>/dev/null || true
                echo "  force-killed (SIGKILL, was stopped) PID $pid on $db: $CMD"
            else
                kill "$pid" 2>/dev/null || true
                sleep 2
                if kill -0 "$pid" 2>/dev/null; then
                    kill -9 "$pid" 2>/dev/null || true
                    echo "  force-killed (SIGKILL) PID $pid on $db: $CMD"
                else
                    echo "  killed (SIGTERM) PID $pid on $db: $CMD"
                fi
            fi
            KILLED=$((KILLED + 1))
        done
    done
    FIXED=$((FIXED + KILLED))
    if [ "$KILLED" -eq 0 ]; then
        echo '  no hung processes found'
    fi

    # 2. Remove stale journal / WAL / SHM files next to each target
    echo '==> Removing stale journal files...'
    JFIXED=0
    for db in "${FILES[@]}"; do
        d="$(dirname "$db")"
        while IFS= read -r -d '' f; do
            rm -f "$f" && echo "  removed $f" || echo "  failed to remove $f"
            JFIXED=$((JFIXED + 1))
        done < <(find "$d" -maxdepth 1 \
            \( -name '.fslckout-journal' -o -name '.fslckout-wal' -o -name '.fslckout-shm' \
            -o -name '*.fossil-journal' -o -name '*.fossil-wal' -o -name '*.fossil-shm' \) -print0 2>/dev/null || true)
    done
    FIXED=$((FIXED + JFIXED))
    if [ "$JFIXED" -eq 0 ]; then
        echo '  no stale journal files found'
    fi

    # 3. Verify every known checkout is now writable
    echo ''
    ALL_OK=1
    for db in "${FILES[@]}"; do
        # Only .fslckout dirs are checkout roots; the $REPO file is not.
        case "$db" in
            */.fslckout)
                d="$(dirname "$db")"
                if (cd "$d" && fossil status >/dev/null 2>&1); then
                    :
                else
                    echo "Warning: $d may still be locked." >&2
                    ALL_OK=0
                fi
                ;;
        esac
    done
    if [ "$ALL_OK" -eq 1 ]; then
        echo "Lock cleared successfully ($FIXED fix(es) applied)."
    else
        echo 'Warning: one or more checkouts may still be locked. Check processes manually.' >&2
        exit 1
    fi

# Mirror trunk history to GitHub (one-way, force push)
sync-github:
   #!/bin/bash
   set -euo pipefail

   FOSSIL_REPO="/mnt/zed/repos/jinn/jinn.fossil"
   MIRROR_DIR="/mnt/zed/repos/jinn/.github-mirror"
   GITHUB_REMOTE="git@github.com:jayson-lennon/jinn.git"

   fossil git export "$MIRROR_DIR" \
       --repository "$FOSSIL_REPO" \
       --mainbranch trunk \
       --autopush "$GITHUB_REMOTE"

# Bump version (major/minor/patch), commit, and tag in Fossil
bump LEVEL:
    #!/usr/bin/env bash
    set -euo pipefail

    # --- Validate input ---
    case "{{LEVEL}}" in
        major|minor|patch) ;;
        *) echo "Usage: just bump <major|minor|patch>" >&2; exit 1 ;;
    esac

    # --- Pre-flight: must be on trunk ---
    BRANCH=$(fossil branch current)
    if [ "$BRANCH" != "trunk" ]; then
        echo "Error: must be on trunk (currently on '$BRANCH')" >&2
        exit 1
    fi

    # --- Pre-flight: working tree must be clean ---
    if [ -n "$(fossil changes --differ)" ]; then
        echo "Error: working tree has uncommitted changes" >&2
        exit 1
    fi

    # --- Pre-flight: PKGBUILD must exist ---
    if [ ! -f PKGBUILD ]; then
        echo "Error: PKGBUILD not found" >&2
        exit 1
    fi

    # --- Compute new version ---
    CURRENT=$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version = "\(.*\)"/\1/p}' Cargo.toml)
    NEW=$(rust-script scripts/bump-version.rs "$CURRENT" "{{LEVEL}}")

    # --- Resolve tag conflicts ---
    CANDIDATE="$NEW"
    ATTEMPTS=0
    while fossil tag list | grep -qx "v${CANDIDATE}"; do
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
    sed -i "s/^pkgver=.*/pkgver=$CANDIDATE/" PKGBUILD
    sed -i "s/^pkgrel=.*/pkgrel=1/" PKGBUILD

    # --- Regenerate Cargo.lock for the new workspace version ---
    cargo update --workspace

    # --- Commit ---
    fossil commit -m "Bump version to ${CANDIDATE}"

    # --- Tag the current checkout ---
    fossil tag add "v${CANDIDATE}" current

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

# Build the Arch package in ./build (isolated from the source tree).
# BUILDDIR keeps makepkg's src/ and pkg/ scratch dirs out of the repo's
# real src/; PKGDEST lands the output tarball in ./build/ instead of root.
# The PKGBUILD uses a local-checkout symlink (prepare()), so makepkg must
# run from the repo root — only the scratch/output dirs are redirected.
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

# Build the release binary and package it into a cargo-binstall tarball.
# The internal layout ({name}-{target}-v{version}/jinn) matches the
# `bin-dir` template in [package.metadata.binstall].
build-release-tarball:
    #!/usr/bin/env bash
    set -euo pipefail

    VERSION=$(sed -n '/^\[workspace\.package\]/,/^[\[]/{s/^version = "\(.*\)"/\1/p}' Cargo.toml)
    TARGET=x86_64-unknown-linux-gnu
    STAGE="jinn-${TARGET}-v${VERSION}"
    TARBALL="${STAGE}.tgz"

    echo "==> Building release binary (target ${TARGET})"
    cargo build --release

    echo "==> Packaging ${TARBALL}"
    STAGE_DIR="$(mktemp -d)"
    mkdir -p "${STAGE_DIR}/${STAGE}"
    cp target/release/jinn "${STAGE_DIR}/${STAGE}/jinn"
    tar -czf "${TARBALL}" -C "${STAGE_DIR}" "${STAGE}"
    rm -rf "${STAGE_DIR}"

    echo "==> Created ${TARBALL}"

# Release the current version to GitHub and verify cargo-binstall.
#
# Orchestrates the full publish flow after `just bump` has been run:
#   1. Mirror trunk (and tags) to GitHub
#   2. Build the cargo-binstall tarball
#   3. Create (or update) the GitHub release and attach the tarball
#   4. Smoke-test: `cargo binstall` into a temp dir, confirm `jinn --version`
#
# Usage: just release v0.98.0
# Prerequisites: gh CLI installed + authenticated, cargo-binstall installed.
release TAG:
    #!/usr/bin/env bash
    set -euo pipefail

    REPO="jayson-lennon/jinn"

    # --- Pre-flight: TAG must match the Cargo.toml version ---
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

    # --- Pre-flight: cargo-binstall must be installed (for smoke test) ---
    if ! command -v cargo-binstall >/dev/null 2>&1; then
        echo "Error: cargo-binstall is not installed." >&2
        echo "  Run: cargo install cargo-binstall" >&2
        exit 1
    fi

    # --- 1. Mirror trunk (and tags) to GitHub ---
    echo '==> Mirroring trunk to GitHub...'
    just sync-github

    # --- 2. Build the cargo-binstall tarball ---
    just build-release-tarball

    TARBALL="jinn-x86_64-unknown-linux-gnu-v${VERSION}.tgz"

    # --- 3. Create the release if it doesn't exist, else upload ---
    if gh release view "{{TAG}}" --repo "${REPO}" >/dev/null 2>&1; then
        echo "==> Uploading ${TARBALL} to existing release {{TAG}}"
        gh release upload "{{TAG}}" "${TARBALL}" --repo "${REPO}" --clobber
    else
        echo "==> Creating release {{TAG}} and uploading ${TARBALL}"
        gh release create "{{TAG}}" "${TARBALL}" --repo "${REPO}" --generate-notes
    fi

    # --- 4. Smoke-test: cargo-binstall into an isolated cargo home ---
    echo '==> Smoke-testing cargo-binstall...'
    SMOKE_HOME="$(mktemp -d)"
    trap 'rm -rf "${SMOKE_HOME}"' EXIT
    CARGO_HOME="${SMOKE_HOME}" cargo binstall \
        --git "https://github.com/${REPO}" \
        --locked jinn \
        --target x86_64-unknown-linux-gnu \
        --no-confirm
    INSTALLED="${SMOKE_HOME}/bin/jinn"
    echo "==> Installed binary reports: $(${INSTALLED} --version)"

    echo "==> Done. https://github.com/${REPO}/releases/tag/{{TAG}}"
