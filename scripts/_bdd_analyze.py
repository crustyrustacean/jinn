#!/usr/bin/env python3
"""Temporary analysis helper: list top-level statements of each flagged test body."""
import sys, re, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bdd_scan as B


def split_statements(body_lines):
    """Yield (start, end) index ranges of top-level statements, treating
    consecutive standalone `//` comment lines as attached to the next stmt."""
    stmts = []
    # Body slice starts at the `fn` signature; skip through its opening brace.
    sig_end = 0
    for idx, l in enumerate(body_lines):
        if B.strip_strings_and_comments(l).rstrip().endswith("{"):
            sig_end = idx
            break
    body_lines = body_lines[sig_end + 1 : -1]  # drop the trailing `}` too
    depth = 0
    start = None
    pend_comments = []
    for idx, l in enumerate(body_lines):
        s = l.strip()
        if not s:
            continue
        if start is None:
            if s.startswith("//"):
                pend_comments.append(idx)
                continue
            start = idx
            depth = 0
        tmp = B.strip_strings_and_comments(l)
        if not tmp.strip():
            continue
        depth += tmp.count("{") + tmp.count("(") + tmp.count("[")
        depth -= tmp.count("}") + tmp.count(")") + tmp.count("]")
        if depth <= 0 and (tmp.rstrip().endswith(";") or tmp.rstrip().endswith("{")):
            stmts.append((pend_comments[0] if pend_comments else start, idx, pend_comments))
            start = None
            pend_comments = []
    if start is not None:
        stmts.append((start, len(body_lines) - 1, []))
    return stmts, sig_end + 1


def main():
    root = "crates/slices/jinn-context-curation"
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in B.SKIP_DIRS]
        for fn in sorted(filenames):
            if not fn.endswith(".rs"):
                continue
            path = os.path.join(dirpath, fn)
            r = B.scan_file(path)
            if not r.findings:
                continue
            names = {f.name for f in r.findings}
            raw = open(path).read()
            code = B.strip_strings_and_comments(raw)
            lines = code.split("\n")
            raw_lines = raw.split("\n")
            print(f"\n########## {path}")
            i = 0
            n = len(lines)
            while i < n:
                s = lines[i].strip()
                if not s.startswith("#["):
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
                if name in names:
                    print(f"--- {name} (base line {base+1}, body {len(blines)} lines)")
                    stmts, off = split_statements(blines)
                    for si, (a, b, pc) in enumerate(stmts, 1):
                        a += off
                        b += off
                        seg = [raw_lines[base + x].rstrip() for x in range(a, b + 1)]
                        pre = [raw_lines[base + x].strip() for x in pc]
                        print(f"    {si:>2} [{base+a+1}] " + ("; ".join(pre) + " >> " if pre else "") + " | ".join(s.strip() for s in seg)[:200])
                i = max(j + 1, code[: end + 1].count("\n"))


if __name__ == "__main__":
    main()
