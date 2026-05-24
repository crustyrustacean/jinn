COPYRIGHT_NAME := "Jayson Lennon"
COPYRIGHT_YEAR := "2026"

export RSTEST_TIMEOUT := "3"

fossil-branch NAME:
    fossil commit -m "Open {{NAME}}" --branch {{NAME}} --allow-empty

test:
    cargo nextest run --workspace --all-features --exclude nullslop-e2e
    cargo test --test e2e -p nullslop-e2e

check:
    cargo check --workspace

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
    cargo test --test e2e -p nullslop-e2e

# Create a new diesel migration
diesel-create NAME:
    diesel migration generate {{NAME}} --migration-dir crates/nullslop-domain/migrations

# Run pending diesel migrations and regenerate schema.rs
diesel-run:
    DATABASE_URL="crates/nullslop-domain/migrations/db.sqlite" diesel migration run --migration-dir crates/nullslop-domain/migrations
    rm -f crates/nullslop-domain/migrations/db.sqlite

# Rollback the last diesel migration and regenerate schema.rs
diesel-rollback:
    DATABASE_URL="crates/nullslop-domain/migrations/db.sqlite" diesel migration rollback --migration-dir crates/nullslop-domain/migrations
    rm -f crates/nullslop-domain/migrations/db.sqlite

# Redo (rollback + re-run) the last diesel migration
diesel-redo:
    DATABASE_URL="crates/nullslop-domain/migrations/db.sqlite" diesel migration redo --migration-dir crates/nullslop-domain/migrations
    rm -f crates/nullslop-domain/migrations/db.sqlite

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

# Copy plugins, themes, and personas to user config directory
install-defaults:
    mkdir -p ~/.config/nullslop/themes
    cp -r themes/*.toml ~/.config/nullslop/themes/
    mkdir -p ~/.config/nullslop/personas
    cp -r personas/*.md ~/.config/nullslop/personas/
    mkdir -p ~/.config/nullslop/prompts
    cp -r prompts/*.md ~/.config/nullslop/prompts/
    @echo "Themes installed to ~/.config/nullslop/themes/"
    @echo "Personas installed to ~/.config/nullslop/personas/"
    @echo "Prompts installed to ~/.config/nullslop/prompts/"

# Report stale Fossil locks (hung processes + stale journal files)
fossil-unlock:
    #!/usr/bin/env bash
    set -euo pipefail

    REPO="$(fossil status 2>/dev/null | sed -n 's/^repository: *//p')"
    if [ -z "$REPO" ]; then
        echo "Error: not inside a Fossil checkout" >&2; exit 1
    fi

    # Resolve symlinks
    REPO="$(readlink -f "$REPO")"
    CHECKOUT_ROOT="$(pwd)"

    FOUND=0

    # 1. Hung Fossil processes
    echo '==> Checking for hung Fossil processes...'
    PIDS=$(lsof "$REPO" 2>/dev/null | awk 'NR>1 && $1=="fossil" {print $2}' | sort -u) || true
    if [ -n "$PIDS" ]; then
        for pid in $PIDS; do
            CMD=$(ps -p "$pid" -o args= 2>/dev/null || echo "<exited>")
            echo "  stale PID $pid: $CMD"
            FOUND=$((FOUND + 1))
        done
    else
        echo "  none found"
    fi

    # 2. Stale journal / WAL / SHM files
    echo '==> Checking for stale journal files...'
    STALE=$(find "$CHECKOUT_ROOT" -maxdepth 3 \
        \( -name '.fslckout-journal' -o -name '.fslckout-wal' -o -name '.fslckout-shm' \
        -o -name '*.fossil-journal' -o -name '*.fossil-wal' -o -name '*.fossil-shm' \) \
        2>/dev/null || true)
    # Also check the repo file itself
    for ext in journal wal shm; do
        f="${REPO}-${ext}"
        [ -f "$f" ] && STALE="$STALE\n$f"
    done
    if [ -n "$STALE" ]; then
        echo -e "$STALE" | while read -r f; do
            [ -z "$f" ] && continue
            echo "  stale: $f"
            FOUND=$((FOUND + 1))
        done
    else
        echo "  none found"
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
    CHECKOUT_ROOT="$(pwd)"

    FIXED=0

    # 1. Kill hung Fossil processes
    PIDS=$(lsof "$REPO" 2>/dev/null | awk 'NR>1 && $1=="fossil" {print $2}' | sort -u) || true
    if [ -n "$PIDS" ]; then
        for pid in $PIDS; do
            OWNER=$(stat -c '%U' "/proc/$pid" 2>/dev/null || echo "")
            ME=$(whoami)
            if [ "$OWNER" != "$ME" ]; then
                echo "  skipping PID $pid (owned by $OWNER, not $ME)"
                continue
            fi
            CMD=$(ps -p "$pid" -o args= 2>/dev/null || echo "<exited>")
            kill "$pid" && echo "  killed PID $pid ($CMD)" || echo "  failed to kill PID $pid"
            FIXED=$((FIXED + 1))
        done
    else
        echo '  no hung processes found'
    fi

    # 2. Remove stale journal / WAL / SHM files
    STALE=$(find "$CHECKOUT_ROOT" -maxdepth 3 \
        \( -name '.fslckout-journal' -o -name '.fslckout-wal' -o -name '.fslckout-shm' \
        -o -name '*.fossil-journal' -o -name '*.fossil-wal' -o -name '*.fossil-shm' \) \
        2>/dev/null || true)
    for ext in journal wal shm; do
        f="${REPO}-${ext}"
        [ -f "$f" ] && STALE="$STALE\n$f"
    done
    if [ -n "$STALE" ]; then
        echo -e "$STALE" | while read -r f; do
            [ -z "$f" ] && continue
            rm -f "$f" && echo "  removed $f" || echo "  failed to remove $f"
            FIXED=$((FIXED + 1))
        done
    else
        echo '  no stale journal files found'
    fi

    # 3. Verify
    echo ''
    if fossil status >/dev/null 2>&1; then
        echo "Lock cleared successfully ($FIXED fix(es) applied)."
    else
        echo 'Warning: database may still be locked. Check for processes manually.' >&2
        exit 1
    fi

# Mirror trunk history to GitHub (one-way, force push)
sync-github:
   #!/bin/bash
   set -euo pipefail

   FOSSIL_REPO="/mnt/zed/repos/nullslop/nullslop.fossil"
   MIRROR_DIR="/mnt/zed/repos/nullslop/.github-mirror"
   GITHUB_REMOTE="git@github.com:jayson-lennon/nullslop.git"

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
    CURRENT=$(grep -m1 '^version' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
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

    # --- Commit ---
    fossil commit -m "Bump version to ${CANDIDATE}"

    # --- Tag the current checkout ---
    fossil tag add "v${CANDIDATE}" current

    echo "Bumped to ${CANDIDATE}, committed and tagged as v${CANDIDATE}"

