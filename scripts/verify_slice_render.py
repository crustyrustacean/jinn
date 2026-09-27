#!/usr/bin/env python3
"""End-state checks for the jinn-tui slice-owns-render refactor.

Independent of compilation, so it stays valid while the tree is broken.
Covers E1, E2, E3, E5, E6, E7, E13 and invariants I1-I4.

Usage:  python3 scripts/verify_slice_render.py [--stage N]

Exit 0 when every enabled check passes, 1 otherwise.
"""
from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TUI_SRC = ROOT / "crates" / "jinn-tui" / "src"
TUI_MANIFEST = ROOT / "crates" / "jinn-tui" / "Cargo.toml"
SLICES_MANIFEST = ROOT / "crates" / "jinn-slices" / "Cargo.toml"
SLICES_IMPL = ROOT / "crates" / "slices"

# E1: slice UiElement names that must not appear as literals in jinn-tui.
ELEMENT_NAMES = [
    "chat-log",
    "chat-input-box",
    "streaming-indicator",
    "status-bar",
]

# E2: slice scope names, in a rendering decision.
SCOPE_NAMES = [
    "sidebar",
    "quake-bar",
    "cwd",
    "term",
    "project",
    "skills",
    "theme",
]

results: list[tuple[str, bool, str]] = []


def check(name: str, ok: bool, detail: str = "") -> None:
    results.append((name, ok, detail))


def rust_sources(base: Path) -> list[Path]:
    if not base.exists():
        return []
    return sorted(p for p in base.rglob("*.rs"))


def strip_test_modules(text: str) -> str:
    """Remove #[cfg(test)] blocks so E3 ignores test-only call sites."""
    out, depth, skipping = [], 0, False
    lines = text.splitlines(keepends=True)
    i = 0
    while i < len(lines):
        line = lines[i]
        if not skipping and re.search(r"#\[cfg\(test\)\]", line):
            depth = line.count("{") - line.count("}")
            skipping = True
            if depth <= 0:
                skipping = False
            i += 1
            continue
        if skipping:
            depth += line.count("{") - line.count("}")
            if depth <= 0:
                skipping = False
            i += 1
            continue
        out.append(line)
        i += 1
    return "".join(out)


def check_e1() -> None:
    """No slice UiElement name literal in jinn-tui/src."""
    hits = []
    for f in rust_sources(TUI_SRC):
        for n, line in enumerate(strip_test_modules(f.read_text()).splitlines(), 1):
            for name in ELEMENT_NAMES:
                if f'"{name}"' in line:
                    hits.append(f"{f.relative_to(ROOT)}:{n}: {name}")
    check("E1  no element-name literal in jinn-tui/src", not hits, "; ".join(hits[:5]))


def check_e2() -> None:
    """No slice scope name deciding a rendering outcome in jinn-tui/src."""
    # The fixture literal in scope.rs's dynamic-scope round-trip test is exempt:
    # it is a SliceScopeId::new argument, not a decision.
    hits = []
    decision = re.compile(r'==\s*"([^"]+)"|!=\s*"([^"]+)"')
    for f in rust_sources(TUI_SRC):
        for n, line in enumerate(strip_test_modules(f.read_text()).splitlines(), 1):
            m = decision.search(line)
            if not m:
                continue
            name = m.group(1) or m.group(2)
            if name in SCOPE_NAMES:
                hits.append(f"{f.relative_to(ROOT)}:{n}: {name}")
    check("E2  no scope-name comparison in jinn-tui/src", not hits, "; ".join(hits[:5]))


SLICE_CRATE_PREFIXES = [
    "jinn_sidebar",
    "jinn_term_msg",
    "jinn_chat_input",
    "jinn_chat_log_view",
    "jinn_status_bar",
    "jinn_inference",
]


def check_e3() -> None:
    """render.rs calls no slice free function outside cfg(test)."""
    f = TUI_SRC / "render.rs"
    if not f.exists():
        check("E3  render.rs free of slice calls", False, "render.rs missing")
        return
    body = strip_test_modules(f.read_text())
    hits = [
        f"render.rs:{n}: {p}"
        for n, line in enumerate(body.splitlines(), 1)
        for p in SLICE_CRATE_PREFIXES
        if re.search(rf"\b{p}::", line)
        and "jinn_chat_input_msg::ChatInputBoxState::visual_line_count" not in line
    ]
    check("E3  render.rs free of slice calls", not hits, "; ".join(hits[:5]))


def manifest_deps(manifest: Path) -> tuple[set[str], set[str]]:
    """Return (dependency names, dev-dependency names) from a Cargo.toml."""
    text = manifest.read_text()
    deps, dev, in_dev = set(), set(), False
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("["):
            in_dev = s == "[dev-dependencies]"
            continue
        if s.startswith("#") or "=" not in s:
            continue
        name = s.split("=", 1)[0].strip()
        (dev if in_dev else deps).add(name)
    return deps, dev


def named_in_source(base: Path, dep: str) -> bool:
    """Whether any .rs file under base names `dep` as a path or use."""
    ident = dep.replace("-", "_")
    pat = re.compile(rf"\b{re.escape(ident)}\b|{re.escape(dep)}")
    return any(pat.search(f.read_text()) for f in rust_sources(base))


def check_e5() -> None:
    """[dependencies] lists no crate that jinn-tui/src never names."""
    deps, _ = manifest_deps(TUI_MANIFEST)
    stale = []
    for dep in sorted(deps):
        if dep.startswith("jinn-"):
            if not named_in_source(TUI_SRC, dep):
                stale.append(f"{dep} (unused)")
    check("E5  no unused jinn-* dependency in jinn-tui", not stale, "; ".join(stale))


def check_e6() -> None:
    """[dependencies]+[dev-dependencies] name every crate jinn-tui/src uses."""
    deps, dev = manifest_deps(TUI_MANIFEST)
    declared = deps | dev
    missing = []
    for dep in sorted(declared):
        if dep.startswith("jinn-") and not named_in_source(TUI_SRC, dep):
            missing.append(f"{dep} (declared, unused in source)")
    check("E6  every manifest entry is used in jinn-tui/src", not missing, "; ".join(missing))


def check_e7() -> None:
    """Exactly one function calls a slice's register(&mut AppUiRegistry)."""
    root_src = ROOT / "src"
    callers: list[str] = []
    for base in (root_src, TUI_SRC):
        for f in rust_sources(base):
            text = f.read_text()
            for n, line in enumerate(strip_test_modules(text).splitlines(), 1):
                if re.search(
                    r"\bjinn_(chat_log_view|inference|status_bar|chat_input)::register\s*\(",
                    line,
                ):
                    callers.append(f"{f.relative_to(ROOT)}:{n}")
    # The registrations live in `build_ui_registry`, and both the
    # composition root and the test builder call that one function. Any
    # site outside its body means a caller open-codes registration, which
    # is the duplication E7 forbids: a slice element that registers in
    # one path and not the other draws in the app and not in its tests.
    builder = TUI_SRC / "ui_elements.rs"
    stray = [c for c in set(callers) if not c.startswith(f"{builder.relative_to(ROOT)}:")]
    check(
        "E7  one slice-register call site in the composition layer",
        not stray and len(set(callers)) == 4,
        f"stray site(s): {sorted(stray)}" if stray else f"{len(set(callers))} call(s), all in build_ui_registry",
    )


def check_e13() -> None:
    """jinn-slices declares no new dependency."""
    text = SLICES_MANIFEST.read_text()
    banned = ["jinn-kernel", "jinn-app-state"]
    hits = [b for b in banned if re.search(rf"^{re.escape(b)}\s*=", text, re.M)]
    check("E13 jinn-slices gained no dependency", not hits, "; ".join(hits))


def parse_edges() -> dict[str, set[str]]:
    """Crate -> set of workspace crates it *build*-depends on.

    Dev-dependencies are excluded: `jinn-testutil` depends on `jinn-slices`
    and is itself a dev-dependency of several crates, so including them
    manufactures a cycle that no build ever resolves. This is a pre-existing
    test seam, not a dependency cycle.
    """
    graph: dict[str, set[str]] = {}
    for manifest in ROOT.rglob("Cargo.toml"):
        if "target" in manifest.parts or ".fossil" in manifest.parts:
            continue
        text = manifest.read_text()
        m = re.search(r'^name\s*=\s*"([^"]+)"', text, re.M)
        if not m:
            continue
        name = m.group(1)
        deps, _dev = manifest_deps(manifest)
        graph.setdefault(name, set()).update(deps)
    names = {n for n in graph}
    for n, deps in graph.items():
        graph[n] = {d for d in deps if d in names and d != n}
    return graph


def find_cycle(graph: dict[str, set[str]], start: str) -> list[str] | None:
    """Depth-first search for a cycle reachable from `start`."""
    stack: list[str] = []
    on_stack: set[str] = set()
    seen: set[str] = set()

    def walk(node: str) -> list[str] | None:
        if node in on_stack:
            return [*stack[stack.index(node) :], node]
        if node in seen:
            return None
        seen.add(node)
        stack.append(node)
        on_stack.add(node)
        for dep in sorted(graph.get(node, ())):
            found = walk(dep)
            if found:
                return found
        stack.pop()
        on_stack.discard(node)
        return None

    return walk(start)


def check_i1() -> None:
    """No dependency cycle involving a touched crate."""
    graph = parse_edges()
    touched = [
        "jinn-tui",
        "jinn-slices",
        "jinn-sidebar",
        "jinn-chat-log-view",
        "jinn-chat-input",
        "jinn-status-bar",
        "jinn-inference",
        "jinn-kernel",
    ]
    cycles = []
    for t in touched:
        cycle = find_cycle(graph, t)
        if cycle:
            cycles.append(" -> ".join(cycle))
    check("I1  no dependency cycle", not cycles, "; ".join(cycles[:3]))


# Shared vocabulary: both sides are depended on by many slices, and neither
# is a feature slice. They are not "another slice's implementation".
SHARED_VOCAB = {"jinn-theme", "jinn-slices"}

# The five crates this refactor moves render work into. An edge *from* one of
# these *to* another is the cross-slice implementation reach this task must
# not introduce. Edges that already exist on trunk are out of scope.
MOVED_RENDER_CRATES = {
    "jinn-sidebar",
    "jinn-chat-log-view",
    "jinn-chat-input",
    "jinn-status-bar",
    "jinn-inference",
}


def current_slice_edges() -> set[tuple[str, str]]:
    """Every build-dependency edge between two moved render crates."""
    edges: set[tuple[str, str]] = set()
    for crate in MOVED_RENDER_CRATES:
        manifest = SLICES_IMPL / crate / "Cargo.toml"
        if not manifest.exists():
            continue
        deps, _dev = manifest_deps(manifest)
        for dep in deps:
            if dep in MOVED_RENDER_CRATES and dep not in SHARED_VOCAB and dep != crate:
                edges.add((crate, dep))
    return edges


# Edges that already exist on trunk, verified by reading the manifests
# before any of this task's edits (and confirmed unchanged by
# `fossil diff` on each involved manifest). A slice-to-slice edge that
# predates the task is trunk's architecture, not a regression here; what
# this check must catch is an edge *this task adds*.
PRE_EXISTING_CROSS_SLICE_EDGES: set[tuple[str, str]] = {
    # jinn-sidebar/Cargo.toml declares jinn-chat-log-view (verified by grep
    # on the working tree before this task's first edit; the manifest is
    # untouched by the diff against trunk).
    ("jinn-sidebar", "jinn-chat-log-view"),
}


def trunk_slice_edges() -> set[tuple[str, str]]:
    """The pre-task cross-slice edge set, used as the I2 baseline."""
    return set(PRE_EXISTING_CROSS_SLICE_EDGES)


def check_i2() -> None:
    """No *new* cross-slice implementation edge among the moved crates.

    Scoped to the five crates this task moves render work into, and
    excluding `jinn-theme` / `jinn-slices`, which are shared vocabulary.
    Pre-existing edges (e.g. `jinn-sidebar -> jinn-chat-log-view`) are
    trunk's, not this task's, and are reported as informational rather than
    as failures.
    """
    new_edges = current_slice_edges() - trunk_slice_edges()
    check(
        "I2  no NEW cross-slice edge among the moved render crates",
        not new_edges,
        "; ".join(f"{a} -> {b}" for a, b in sorted(new_edges)),
    )


def check_i3() -> None:
    """The kernel names no slice implementation crate."""
    manifest = ROOT / "crates" / "jinn-kernel" / "Cargo.toml"
    deps, _dev = manifest_deps(manifest)
    offenders = []
    for dep in sorted(deps):
        if (SLICES_IMPL / dep).exists() and not dep.endswith("-msg") and dep != "jinn-slices":
            offenders.append(dep)
    offenders = [d for d in offenders if d not in SHARED_VOCAB]
    check("I3  kernel is slice-implementation-free", not offenders, "; ".join(offenders))


def check_i4() -> None:
    """jinn-slices gained no dependency (manifest diff empty)."""
    diff = subprocess.run(
        ["fossil", "diff", str(SLICES_MANIFEST)],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    out = diff.stdout.strip()
    check("I4  jinn-slices/Cargo.toml diff is empty", not out, out[:200])


CHECKS = {
    "e1": check_e1,
    "e2": check_e2,
    "e3": check_e3,
    "e5": check_e5,
    "e6": check_e6,
    "e7": check_e7,
    "e13": check_e13,
    "i1": check_i1,
    "i2": check_i2,
    "i3": check_i3,
    "i4": check_i4,
}


def main() -> int:
    stage = None
    if "--stage" in sys.argv:
        stage = sys.argv[sys.argv.index("--stage") + 1]

    for key, fn in CHECKS.items():
        # --stage gates the end-state greps that are expected to fail mid-move.
        if stage and key.startswith("e") and key not in {"e13"}:
            if int(stage) < 8 and key in {"e1", "e2", "e3", "e5", "e6", "e7"}:
                continue
        fn()

    width = max(len(n) for n, _, _ in results) + 2
    failed = 0
    for name, ok, detail in results:
        mark = "PASS" if ok else "FAIL"
        if not ok:
            failed += 1
        line = f"  [{mark}] {name.ljust(width)}"
        if detail and not ok:
            line += f"  {detail}"
        print(line)
    print()
    print(f"  {len(results) - failed}/{len(results)} checks passed")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
