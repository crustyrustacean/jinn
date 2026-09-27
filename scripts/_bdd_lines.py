#!/usr/bin/env python3
"""Temporary helper: locate Given/When/Then anchor lines inside test bodies.

Splits each test body into top-level statements using a character-level
depth scanner over the comment/string-stripped source, then reports the
statements so the When (last setup statement before the first assertion) and
Then (first assertion) anchors can be placed precisely.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bdd_scan as B


def find_tests(raw: str, code: str):
    """Yield (name, base, blines, body_end) for each test function."""
    lines = code.split("\n")
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
        sig_line = None
        while k < n and k < j + 12:
            m = B.FN.match(lines[k])
            if m:
                name = m.group(1)
                sig_line = k
                break
            k += 1
        if name is None:
            i = j + 1
            continue
        pos = sum(len(l) + 1 for l in lines[:sig_line])
        brace = code.find("{", pos)
        found = B.find_body(code, brace)
        if not found:
            i = sig_line + 1
            continue
        body, end = found
        base = code[:brace].count("\n")
        yield name, base, body.split("\n"), (j, sig_line, end)
        i = max(j + 1, code[: end + 1].count("\n"))


def split_statements(blines):
    """Top-level statements as (start, end) body-relative line indices.

    Character-level depth scan: a statement ends at a `;` seen at depth 0, or
    at the `}` that closes a depth-0-opened block when no `;` follows.
    """
    stmts = []
    n = len(blines)
    sig_end = 0
    for idx, l in enumerate(blines):
        if B.strip_strings_and_comments(l).rstrip().endswith("{"):
            sig_end = idx
            break
    i = sig_end + 1
    depth = 0
    start_line = None
    while i < n - 1:  # last line is the closing `}`
        line = blines[i]
        start_line = i if start_line is None else start_line
        code = B.strip_strings_and_comments(line)
        for ch in code:
            if ch in "([{":
                depth += 1
            elif ch in ")]}":
                depth -= 1
                if depth == 0 and ch == "}":
                    # Block closed at top level: the statement ends here
                    # unless a `;` follows on a later line.
                    nxt = B.strip_strings_and_comments("\n".join(blines[i : i + 2])).rstrip()
                    if not nxt.endswith(";"):
                        stmts.append((start_line, i))
                        start_line = None
                        break
                elif depth < 0:
                    # Closed the test body itself.
                    stmts.append((start_line, i - 1))
                    return stmts, sig_end + 1
        if start_line is not None and depth == 0 and code.rstrip().endswith(";"):
            stmts.append((start_line, i))
            start_line = None
        i += 1
    return stmts, sig_end + 1


ASSERT_START = re.compile(r"^(assert|debug_assert)")


def main():
    for path in sys.argv[1:]:
        if not path.endswith(".rs"):
            for dp, dn, fns in os.walk(path):
                dn[:] = [d for d in dn if d not in B.SKIP_DIRS]
                for f in sorted(fns):
                    if f.endswith(".rs"):
                        report(os.path.join(dp, f))
            continue
        report(path)


def report(path):
    raw = open(path, encoding="utf-8").read()
    code = B.strip_strings_and_comments(raw)
    raw_lines = raw.split("\n")
    print(f"\n########## {path}")
    for name, base, blines, _ in find_tests(raw, code):
        stmts, off = split_statements(blines)
        print(f"--- {name}  (fn @ {base+1}, body {len(blines)} lines)")
        for a, b in stmts:
            text = []
            for x in range(a, b + 1):
                s = raw_lines[base + x].strip()
                if s and not s.startswith("//"):
                    text.append(s)
            t = " ".join(text)
            if len(t) > 150:
                t = t[:150] + "..."
            print(f"    {base+a+1:>5}-{base+b+1:<5} {'ASSERT' if ASSERT_START.match(t) else '      '} {t}")


if __name__ == "__main__":
    main()
