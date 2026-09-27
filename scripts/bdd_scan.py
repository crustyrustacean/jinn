#!/usr/bin/env python3
"""Structural scan for BDD conformance across the workspace's test code.

Measures four violation classes per test function:

  missing_bdd  -- no `// Given`, `// When`, and `// Then` inside the body,
                  in that order.
  multi_when   -- more than one `// When` comment in the body.
  long_body    -- body longer than MAX_BODY_LINES lines.
  dup_name     -- a test name used by more than one function in the same
                  file/module, so a failing run is ambiguous.

A fifth figure, `assert_counts`, is reported as INFORMATION ONLY and is not a
violation. A roundtrip test asserting id, title, and entry count is one
behaviour, not three -- counting its assertions would manufacture thousands
of false positives. See Decision Rule 1 in the task contract.

Handles rstest attribute stacking: `#[rstest::rstest]` above `#[test]` or
`#[tokio::test]` is ONE test function, not two.

Usage:
    scripts/bdd_scan.py [PATH ...]        # default: whole workspace
    scripts/bdd_scan.py crates/slices/jinn-mcp
    scripts/bdd_scan.py --json           # machine-readable totals

This script MEASURES. It is deliberately not wired into the justfile or any
CI path -- see the task contract's non-goals.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from dataclasses import dataclass, field

SKIP_DIRS = {"target", "vendor", ".git", "node_modules"}
MAX_BODY_LINES = 60

# `#[test]`, `#[tokio::test]`, `#[tokio::test(flavor = "...")]`,
# `#[rstest::rstest]`, `#[rstest]`, `#[rstest::rstest(...)]`.
# Matched against a single stripped line; the attribute region walk below
# consumes multi-line attribute groups separately.
TEST_ATTR = re.compile(
    r"#\[(?:test|tokio::test(?:\([^()]*\))?|rstest(?:::\w+)*(?:\([^()]*\))?)\]"
)
GIVEN = re.compile(r"//\s*Given\b")
WHEN = re.compile(r"//\s*When\b")
THEN = re.compile(r"//\s*Then\b")
ASSERT = re.compile(r"\b(?:assert|assert_eq|assert_ne|assert!|debug_assert\w*)\s*[!(]")

FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)\s*[\(<]")


@dataclass
class Finding:
    kind: str
    name: str
    line: int


@dataclass
class FileReport:
    path: str
    tests: int = 0
    violations: dict[str, int] = field(
        default_factory=lambda: {
            "missing_bdd": 0,
            "multi_when": 0,
            "long_body": 0,
            "dup_name": 0,
        }
    )
    findings: list[Finding] = field(default_factory=list)
    seen_names: dict[str, int] = field(default_factory=dict)
    post_when_asserts: int = 0
    multi_assert: int = 0


def strip_strings_and_comments(src: str) -> str:
    """Blank out string/char literals, line comments, and block comments.

    Line and block structure is preserved exactly, so line N of the result
    still corresponds to line N of the source. Comment text is blanked here;
    the caller re-reads the ORIGINAL lines when it needs `// When`, which is
    why `scan_file` keeps both `raw` and the stripped `code`.
    """
    out: list[str] = []
    i = 0
    line = 1
    n = len(src)
    while i < n:
        ch = src[i]
        if ch == "\n":
            out.append("\n")
            line += 1
            i += 1
        elif ch == "/" and i + 1 < n and src[i + 1] == "/":
            while i < n and src[i] != "\n":
                out.append(" ")
                i += 1
        elif ch == "/" and i + 1 < n and src[i + 1] == "*":
            depth = 0
            while i < n:
                if src[i] == "/" and i + 1 < n and src[i + 1] == "*":
                    depth += 1
                    out.append("  ")
                    i += 2
                elif src[i] == "*" and i + 1 < n and src[i + 1] == "/":
                    depth -= 1
                    out.append("  ")
                    i += 2
                    if depth == 0:
                        break
                else:
                    out.append("\n" if src[i] == "\n" else " ")
                    if src[i] == "\n":
                        line += 1
                    i += 1
        elif ch == '"':
            out.append(" ")
            i += 1
            while i < n and src[i] != '"':
                if src[i] == "\\":
                    # Emit the backslash as a space, but KEEP a following
                    # newline as a newline: a trailing `\` inside a string
                    # literal is a line continuation, and turning it into a
                    # space would shift every subsequent line by one and
                    # desynchronise this stripped text from the original.
                    out.append(" ")
                    i += 1
                    if i < n:
                        out.append("\n" if src[i] == "\n" else " ")
                        if src[i] == "\n":
                            line += 1
                        i += 1
                    continue
                if src[i] == "\n":
                    out.append("\n")
                    line += 1
                else:
                    out.append(" ")
                i += 1
            if i < n:
                out.append(" ")
                i += 1
        elif ch == "'":
            # Char literal or lifetime. Only treat as literal if it closes
            # within a few characters; `'a` is a lifetime.
            m = re.match(r"'(\\.|[^'\\])'", src[i : i + 4])
            if m:
                out.append(" " * len(m.group(0)))
                i += len(m.group(0))
            else:
                out.append("'")
                i += 1
        else:
            out.append(ch)
            i += 1
    return "".join(out)


def find_body(code: str, open_brace: int) -> tuple[str, int] | None:
    """Return (body_source, end_index) for the brace block starting at
    `open_brace`, or None if unbalanced."""
    depth = 0
    i = open_brace
    n = len(code)
    while i < n:
        c = code[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return code[open_brace : i + 1], i
        i += 1
    return None


def scan_file(path: str) -> FileReport:
    with open(path, encoding="utf-8") as f:
        raw = f.read()

    code = strip_strings_and_comments(raw)
    report = FileReport(path=path)

    lines = code.split("\n")
    raw_lines = raw.split("\n")

    # Walk lines. When a test attribute region is found, locate the fn it
    # decorates, count it ONCE, and skip past its body.
    i = 0
    n = len(lines)
    while i < n:
        s = lines[i].strip()
        if not s.startswith("#["):
            i += 1
            continue

        # Consume the contiguous attribute region (multi-line safe).
        depth = 0
        has_test_attr = False
        j = i
        while j < n:
            sj = lines[j].strip()
            depth += sj.count("[") + sj.count("(")
            depth -= sj.count("]") + sj.count(")")
            if TEST_ATTR.fullmatch(sj):
                has_test_attr = True
            if depth <= 0:
                nxt = j + 1
                if nxt < n and lines[nxt].strip().startswith("#["):
                    j = nxt
                    continue
                break
            j += 1
        else:
            j = n - 1

        if not has_test_attr:
            i = j + 1
            continue

        # Find the decorated `fn`, allowing a `pub` / `async` / return-type
        # prefix and possibly a where-clause before the body brace.
        k = j + 1
        fn_line = None
        name = None
        while k < n and k < j + 12:
            m = FN.match(lines[k])
            if m:
                fn_line = k
                name = m.group(1)
                break
            if lines[k].strip().startswith("#["):
                break
            k += 1

        if fn_line is None or name is None:
            i = j + 1
            continue

        # Locate the body's opening brace: the first `{` at or after the fn
        # signature line. `code` and `raw` have identical line structure, so a
        # character offset in `code` maps to the same LINE index in both.
        pos = sum(len(l) + 1 for l in lines[:fn_line])
        brace = code.find("{", pos)
        if brace == -1:
            i = fn_line + 1
            continue

        found = find_body(code, brace)
        if found is None:
            i = fn_line + 1
            continue
        body, end = found

        report.tests += 1

        # The body block spans line `base` (the line holding its opening
        # brace) through the closing brace. Index both the stripped and the
        # original text through it: stripped for braces/asserts, original
        # for the `// When` comment that the stripper blanked out.
        base = code[:brace].count("\n")
        n_body_lines = body.count("\n") + 1

        def raw_line(idx: int) -> str:
            real = base + idx
            return raw_lines[real] if 0 <= real < len(raw_lines) else ""

        def code_line(idx: int) -> str:
            real = base + idx
            return lines[real] if 0 <= real < len(lines) else ""

        flags: list[str] = []

        given_at = when_at = then_at = None
        for idx in range(n_body_lines):
            t = raw_line(idx)
            if given_at is None and GIVEN.search(t):
                given_at = idx
            if when_at is None and WHEN.search(t):
                when_at = idx
            if then_at is None and THEN.search(t):
                then_at = idx

        if given_at is None or when_at is None or then_at is None:
            flags.append("missing_bdd")
        elif not (given_at < when_at < then_at):
            flags.append("missing_bdd")

        whens = sum(1 for idx in range(n_body_lines) if WHEN.search(raw_line(idx)))
        if whens > 1:
            flags.append("multi_when")

        if n_body_lines > MAX_BODY_LINES:
            flags.append("long_body")

        # INFORMATION ONLY -- not a violation. See Decision Rule 1.
        post = sum(
            1
            for idx in range(when_at if when_at is not None else 0, n_body_lines)
            if ASSERT.search(code_line(idx))
        )
        report.post_when_asserts += post
        if post > 1:
            report.multi_assert += 1

        # A name reused inside one file makes a failing run ambiguous.
        report.seen_names[name] = report.seen_names.get(name, 0) + 1
        if report.seen_names[name] > 1:
            flags.append("dup_name")

        for kind in flags:
            report.violations[kind] += 1
            report.findings.append(Finding(kind, name, base + 1))

        # Resume scanning on the line after the closing brace.
        i = max(j + 1, code[: end + 1].count("\n"))

    return report


def crate_of(path: str) -> str:
    """Crate name for a source path, e.g. `slices/jinn-mcp/src/a.rs` -> `jinn-mcp`."""
    parts = path.replace("\\", "/").split("/")
    for idx, p in enumerate(parts):
        if p == "crates" and idx + 1 < len(parts):
            if parts[idx + 1] == "slices" and idx + 2 < len(parts):
                return parts[idx + 2]
            return parts[idx + 1]
    return "(root)"


def collect(paths: list[str]) -> list[str]:
    files: list[str] = []
    for p in paths:
        if os.path.isfile(p):
            files.append(p)
            continue
        for dirpath, dirnames, filenames in os.walk(p):
            dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
            for fn in filenames:
                if fn.endswith(".rs"):
                    files.append(os.path.join(dirpath, fn))
    return sorted(files)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("paths", nargs="*", default=None)
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--verbose", "-v", action="store_true")
    ap.add_argument("--crate", default=None, help="filter output to one crate")
    args = ap.parse_args()

    paths = args.paths or [os.getcwd()]
    reports = [scan_file(f) for f in collect(paths)]

    totals = {"missing_bdd": 0, "multi_when": 0, "long_body": 0, "dup_name": 0}
    flagged_files = 0
    total_tests = 0
    flagged_tests: set[tuple[str, str]] = set()
    by_crate: dict[str, int] = {}
    info = {"tests_with_multiple_post_when_asserts": 0, "total_post_when_asserts": 0}

    for r in reports:
        if r.tests == 0:
            continue
        total_tests += r.tests
        info["tests_with_multiple_post_when_asserts"] += r.multi_assert
        info["total_post_when_asserts"] += r.post_when_asserts
        fv = sum(r.violations.values())
        if fv:
            flagged_files += 1
        for k in totals:
            totals[k] += r.violations[k]
        for f in r.findings:
            flagged_tests.add((r.path, f.name))
        if fv:
            by_crate[crate_of(r.path)] = by_crate.get(crate_of(r.path), 0) + fv

    summary = {
        "files_scanned": len(reports),
        "test_functions": total_tests,
        "flagged_files": flagged_files,
        "flagged_tests": len(flagged_tests),
        "violations": totals,
        "violations_total": sum(totals.values()),
        "by_crate": dict(sorted(by_crate.items(), key=lambda kv: -kv[1])),
        "info": info,
    }

    if args.json:
        print(json.dumps(summary, indent=2))
        return 0

    print(f"files scanned     : {summary['files_scanned']}")
    print(f"test functions    : {total_tests}")
    print(f"flagged tests     : {len(flagged_tests)}  in {flagged_files} files")
    print("violations by class:")
    for k, v in totals.items():
        print(f"    {k:<12} {v}")
    print(f"    {'TOTAL':<12} {summary['violations_total']}")
    print("information (not violations):")
    print(f"    tests with >1 post-When assert : {info['tests_with_multiple_post_when_asserts']}")
    print(f"    total post-When asserts        : {info['total_post_when_asserts']}")
    print("\nflagged per crate:")
    for c, v in summary["by_crate"].items():
        print(f"    {v:>5}  {c}")

    if args.verbose:
        print("\ndetail:")
        for r in reports:
            if not r.findings:
                continue
            if args.crate and crate_of(r.path) != args.crate:
                continue
            print(f"  {r.path}  ({r.tests} tests)")
            for f in r.findings:
                print(f"      {f.kind:<12} {f.name}  @{f.line}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
