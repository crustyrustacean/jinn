COPYRIGHT_NAME := "Jayson Lennon"
COPYRIGHT_YEAR := "2026"

export RSTEST_TIMEOUT := "3"

fossil-branch NAME:
    fossil commit -m "Open {{NAME}}" --branch {{NAME}} --allow-empty

test:
    cargo nextest run --workspace --all-features --exclude nullslop-e2e --exclude llm
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
    just lint-testlength

# Full CI pipeline (lint + test + docs)
ci: lint test
    cargo test --workspace --doc --exclude llm
    cargo doc --workspace --no-deps

# Run all cucumber tests
cucumber:
    cargo test --test e2e -p nullslop-e2e

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

# Copy plugins and themes to user config directory
install-defaults:
    mkdir -p ~/.config/nullslop/themes
    cp -r themes/*.toml ~/.config/nullslop/themes/
    @echo "Themes installed to ~/.config/nullslop/themes/"

# Mirror trunk history to GitHub (one-way, force push)
sync-github:
   #!/bin/bash
   set -euo pipefail

   FOSSIL_REPO="/mnt/zed/repos/nullslop2/nullslop.fossil"
   MARKS_FILE="/mnt/zed/repos/nullslop2/.git-fossil-marks"
   GITHUB_REMOTE="git@github.com:jayson-lennon/nullslop.git"

   TMPDIR=$(mktemp -d)
   trap "rm -rf $TMPDIR" EXIT

   echo "Initializing temp bare repo..."
   git init --bare "$TMPDIR/repo"
   cd "$TMPDIR/repo"

   git config user.name "Jayson Lennon"
   git config user.email "jayson@jaysonlennon.dev"

   EXPORT_ARGS="--repository $FOSSIL_REPO --git --export-marks $TMPDIR/new-marks"
   if [ -f "$MARKS_FILE" ]; then
       echo "Incremental export (marks file found)..."
       EXPORT_ARGS="$EXPORT_ARGS --import-marks $MARKS_FILE"
   else
       echo "Full export (no marks file yet)..."
   fi

   echo "Exporting from Fossil..."
   fossil export $EXPORT_ARGS | git fast-import

   echo "Pushing to GitHub..."
   git remote add origin "$GITHUB_REMOTE"
   git push --force origin trunk

   # Only persist marks after successful push
   mv "$TMPDIR/new-marks" "$MARKS_FILE"
   echo "Sync complete."

