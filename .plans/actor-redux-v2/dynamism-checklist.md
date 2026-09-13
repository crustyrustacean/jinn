# Dynamism Checklist — the per-port WASM-readiness invariant

Every slice ported under the actor-redux-v2 framework must keep this
checklist true. It is an invariant, not code: nothing here is built
until the first WASM guest is authored (per plan decision 7). The
checklist exists so that day never requires reworking an in-tree slice.

A slice is **dynamism-clean** when every item holds. A port that breaks
an item is not done.

## 1. Dynamic slot keys

Slice cells are minted under `SlotKey`s that are data (constructed from
string ids), not statics reachable only from kernel code. A guest
manifest names slots; the host resolves them.

- Check: `SlotKey::builtin(slice_name, slot)` calls take names from the
  slice's own constants — no hardcoded kernel-side key literals.

## 2. Data-carried scopes and intents

Route rows and input hooks carry scope/intent data, not captures of
kernel enums. `BindSite::StaticScopes(&[...])` is data; dynamic scopes
register by value.

- Check: a slice's rows can be constructed from a manifest description
  (route id, scope, key, outcome) without kernel type context.

## 3. Runtime-registered routes

Forward/reverse bridge routes stage at activation
(`SliceHost::forward`/`reverse`) — nothing about a crossing message is
compile-time tabled in the kernel. Relays spawn per route from the
staged manifest.

- Check: commenting the slice's `activate()` removes its routes; no
  kernel bridge table mentions the slice's messages.

## 4. Dynamic config sections

Slices read config through `config_section::<T>` (typed) or
`config_section_value` (dynamic raw-TOML face — the exact shape a guest
receives over the wire). Defaults are slice-supplied; validation runs
at activation.

- Check: the slice's config struct derives `Deserialize` + `Default`
  and lives in the slice (after its crate cut), not in kernel config
  structs.

## 5. Views are pure functions

Views stay `(&slice_state, theme) -> output` — no captures, no kernel
state reads beyond the passed context (per
`.plans/declarative-ui/plan.md`; rendering is frozen during this
campaign).

- Check: the view type implements `SliceView` with no kernel
  dependency beyond `jinn-slices` + `jinn-theme` + `ratatui`.

## 6. Removability

Deleting (or commenting) the slice's `activate()` call removes the
slice completely — cells, actors, routes, rows, views, tabs, overlays,
config validation — with no other edits. Composition compiles and
launches without it.

- Check: performed once per port, recorded in the port's phase review.

## 7. No kernel residue

After a slice's crate cut, `grep` over kernel code (jinn-domain minus
slice crates, plus root `src/`) finds the slice's domain names only at
composition's `activate()` call (and the gateway/frontend crate for
EXPORT slices).

- Check: `grep -ri <slice>`; every hit is either composition, the
  slice crate itself, or its contracts crate.
