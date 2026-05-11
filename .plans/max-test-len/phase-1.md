# Phase 1: Add `lint-testlength` recipe to justfile

## Problem

No mechanism exists to detect oversized inline test modules. We need a `just lint-testlength` recipe that scans all `.rs` files (excluding `vendor/`), finds inline `#[cfg(test)] mod name { ... }` blocks over 200 lines, and reports them as warnings.

## What Moves / What Stays

- **Modifies**: `justfile` — add one new recipe
- **Stays**: Everything else unchanged

## File Changes

### 1. `justfile` — add `lint-testlength` recipe

Add after the `apply-license` recipe. Uses the shebang `#!/usr/bin/env python3` pattern (similar to `apply-license` which uses `#!/bin/bash`).

```python
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
                   print(f"WARN: {relpath}:{i + 1}: module is {mod_lines} lines (max {max_lines})")
                   found += 1

   if found:
       print(f"\n{found} inline test module(s) exceed {max_lines} lines")
```

## Implementation Order

1. Add the recipe to `justfile`
2. Run `just lint-testlength` and verify it produces WARN output

## Acceptance Criteria

- [x] `just lint-testlength` runs without error
- [x] Output uses `WARN: file:line: module is N lines (max 200)` format
- [x] `vendor/` directory is excluded
- [x] Only inline test modules are detected (not `mod tests;`)
- [x] Exit code is always 0
