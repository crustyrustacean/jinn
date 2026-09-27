#!/usr/bin/env bash
# One-shot mechanical rename: jinn-domain -> jinn-kernel, jinn_domain -> jinn_kernel.
#
# Scope is every tracked text file except the ones a human hand-edits:
#   CHANGELOG.md      human-authored only, never touched
#   vendor/           out of scope (project style guide); verified zero hits
#   target/, .fslckout   build artifacts and the fossil checkout database
#
# Two substitutions, in this order, so `jinn-domain` becomes `jinn-kernel` and
# never gets re-matched by the second rule. Nothing else is rewritten: this
# script performs no sentence rewriting, only identifier/path substitution.
set -euo pipefail

DRY_RUN=0
if [ "${1:-}" = "--dry-run" ]; then
    DRY_RUN=1
fi

# Files carrying the name, excluding the paths above.
mapfile -t FILES < <(
    grep -rIl "jinn[-_]domain" . \
        --exclude-dir=target \
        --exclude-dir=vendor \
        --exclude-dir=.git \
        --exclude=CHANGELOG.md \
        --exclude=.fslckout \
    | sed 's|^\./||' | sort
)

echo "candidate files: ${#FILES[@]}"

SITES=0
for f in "${FILES[@]}"; do
    n=$(grep -c "jinn[-_]domain" "$f" || true)
    SITES=$((SITES + n))
    if [ "$DRY_RUN" -eq 1 ]; then
        printf '%6s  %s\n' "$n" "$f"
    else
        sed -i -e 's/jinn-domain/jinn-kernel/g' -e 's/jinn_domain/jinn_kernel/g' "$f"
    fi
done

echo "total sites: $SITES"

if [ "$DRY_RUN" -eq 0 ]; then
    echo "--- residual check (excluding target/, vendor/, .fslckout, CHANGELOG.md):"
    grep -rIl "jinn[-_]domain" . \
        --exclude-dir=target \
        --exclude-dir=vendor \
        --exclude-dir=.git \
        --exclude=CHANGELOG.md \
        --exclude=.fslckout || echo "  (none)"
fi
