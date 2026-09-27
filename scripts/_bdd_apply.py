#!/usr/bin/env python3
"""Temporary helper: insert Given/When/Then comments into test bodies.

Splits each test body into a Given region (every top-level statement up to and
including the last one whose statement text contains a call — a `let` with a
call, an assignment with a call, a method call on a local, a bare macro call)
and a Then region (everything after that statement: asserts and result
inspections).

Existing BDD comments are preserved verbatim. When the body already has a
`// When` (or `// Then`), the When anchor is used instead of the heuristic.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bdd_scan as B
from _bdd_analyze import split_statements

LET_CALL = re.compile(r"^let\b.*\w\s*[({=]")


def looks_like_call(text: str) -> bool:
    """True when the statement is a fixture construction / mutation rather
    than a plain binding or an assertion."""
    t = text.strip()
    if not t or t.startswith(("let mut", "mut ")):
        return False
    if t.startswith("let "):
        return bool(LET_CALL.match(t))
    if t.startswith(("//", "assert", "for ", "if ", "let ")):
        return False
    return True


def statement_text(body_lines, a, b, base, raw_lines):
    parts = []
    for x in range(a, b + 1):
        line = raw_lines[base + x]
        s = line.strip()
        if not s or s.startswith("//"):
            continue
        parts.append(s)
    return " ".join(parts)


def plan_test(body_lines, base, raw_lines):
    """Return (given_end, when_idx, then_idx) as body-relative indices, or None
    when the body already carries BDD comments or cannot be planned."""
    joined = "\n".join(body_lines)
    if B.GIVEN.search(joined) or B.WHEN.search(joined) or B.THEN.search(joined):
        return None
    stmts, off = split_statements(body_lines)
    if not stmts:
        return None
    texts = [statement_text(body_lines, a, b, base, raw_lines) for (a, b, _pc) in stmts]
    # Last statement that constructs a fixture or performs a mutation.
    when_rel = None
    for (i, t) in enumerate(texts):
        if t.startswith("assert"):
            continue
        if looks_like_call(t):
            when_rel = stmts[i][0] + off
    if when_rel is None:
        # No executable call: place When immediately before the first statement.
        when_rel = stmts[0][0] + off
    # Given region: from the first statement through the line just before When.
    first_rel = stmts[0][0] + off
    # Then comment goes right after the When statement ends.
    idx = next(i for i, (a, b, _p) in enumerate(stmts) if a + off == when_rel)
    then_rel = stmts[idx][1] + off + 1
    return (first_rel, when_rel, then_rel)


def indent_of(line):
    return line[: len(line) - len(line.lstrip())]


def apply_to_file(path, dry=False):
    raw = open(path, encoding="utf-8").read()
    code = B.strip_strings_and_comments(raw)
    lines = code.split("\n")
    raw_lines = raw.split("\n")
    edits = []
    i = 0
    n = len(lines)
    while i < n:
        if not lines[i].strip().startswith("#["):
            i += 1
            continue
        depth = 0
        has_test = False
        j = i
        while j < n:
            sj = lines[j].strip()
            depth += sj.count("[") + sj.count("(")
            depth -= sj.count("]") + sj.count(")")
            if B.TEST_ATTR.fullmatch(sj):
                has_test = True
            if depth <= 0:
                nxt = j + 1
                if nxt < n and lines[nxt].strip().startswith("#["):
                    j = nxt
                    continue
                break
            j += 1
        if not has_test:
            i = j + 1
            continue
        k = j + 1
        name = None
        while k < n and k < j + 12:
            m = B.FN.match(lines[k])
            if m:
                name = m.group(1)
                break
            k += 1
        if name is None:
            i = j + 1
            continue
        pos = sum(len(l) + 1 for l in lines[:k])
        brace = code.find("{", pos)
        found = B.find_body(code, brace)
        if not found:
            i = k + 1
            continue
        body, end = found
        base = code[:brace].count("\n")
        blines = body.split("\n")
        plan = plan_test(blines, base, raw_lines)
        if plan is None:
            i = max(j + 1, code[: end + 1].count("\n"))
            continue
        first_rel, when_rel, then_rel = plan
        ind = indent_of(raw_lines[base + 1])
        name_line = next(
            (x for x in range(len(blines)) if B.FN.search(blines[x])), None
        )
        if name_line is None:
            i = max(j + 1, code[: end + 1].count("\n"))
            continue
        given_at = name_line + 1
        edits.append((base + given_at, ind + "// Given <placeholder>"))
        edits.append((base + when_rel, ind + "// When <placeholder>"))
        edits.append((base + then_rel, ind + "// Then <placeholder>"))
        i = max(j + 1, code[: end + 1].count("\n"))

    if not edits:
        return 0
    if not dry:
        for line_no, text in sorted(edits, reverse=True):
            raw_lines.insert(line_no, text)
        open(path, "w", encoding="utf-8").write("\n".join(raw_lines))
    return len(edits) // 3


def main():
    targets = sys.argv[1:]
    total = 0
    for root in targets:
        files = []
        if os.path.isfile(root):
            files = [root]
        else:
            for dp, dn, fns in os.walk(root):
                dn[:] = [d for d in dn if d not in B.SKIP_DIRS]
                files += [os.path.join(dp, f) for f in fns if f.endswith(".rs")]
        for f in sorted(files):
            c = apply_to_file(f, dry="--dry" in sys.argv)
            if c:
                print(f"{f}: {c}")
            total += c
    print(f"TOTAL {total}")


main()
