//! Slice keybind generation — the bridge between slice route rows and
//! the which-key keymap.
//!
//! Slices declare their keybinds as [`RouteRow`]s (scope + key +
//! outcome) attached to `KeyRoutes` at activation. This module
//! materializes those rows as keymap bindings, once, after all
//! activations: static intents resolve through the `RouteId` → intent
//! table below, dynamic actions bind to `Intent::Dynamic` carrying the
//! row's `(slice, action)` identity. An unregistered slice's keys are
//! simply never bound — removability is automatic, not maintained.

use jinn_domain::Key;
use jinn_domain::KeyEvent;
use jinn_slices::SliceScopeId;
use jinn_slices::route::BindSite;
use jinn_slices::route::KeyRoutes;
use jinn_slices::route::RouteOutcome;
use jinn_slices::route::RouteRow;
use ratatui_which_key::Keymap;
use ratatui_which_key::parse_key_sequence;

use crate::keymap::KeyCategory;
use crate::scope::Scope;
use jinn_domain::KernelIntent;

/// Resolves a static row's [`RouteId`] to the composition intent it
/// binds. Slice keybind blocks used to hardcode these — the table is
/// now the single central record of slice keys that are plain static
/// intents (shared-chrome keys like `q` → quit).
fn static_intent(route_id: &str) -> Option<KernelIntent> {
    match route_id {
        "dashboard:quit" | "sidebar:quit" | "term:quit" => Some(KernelIntent::Quit),
        "dashboard:switch-tab" => Some(KernelIntent::SwitchTab),
        "dashboard:which-key" | "sidebar:which-key" | "term:which-key" => {
            Some(KernelIntent::ToggleWhichkey)
        }
        "quake-bar:ctrl-clear" | "sidebar:ctrl-clear" => Some(KernelIntent::CtrlClear),
        _ => None,
    }
}

/// Maps a row category hint onto the keymap category enum.
/// Binds a picker spec's declared rows into its static scope as
/// data-carried [`Intent::PickerAction`] bindings.
fn category(name: &str) -> KeyCategory {
    match name {
        "navigation" => KeyCategory::Navigation,
        "input" => KeyCategory::Input,
        _ => KeyCategory::General,
    }
}

/// The keymap scope a row binds into.
///
/// `OwnScope` rows bind in their slice's dynamic scope; `GlobalToggle`
/// rows bind in every static scope and in other slices' dynamic scopes
/// (input-hook scopes included, key-hook scopes deliberately excluded
/// so capture stays hermetic); `StaticScopes` rows bind in the named
/// composition scopes, looked up by display name.
fn scopes_for_row<'a>(
    routes: &'a KeyRoutes,
    row: &'a RouteRow,
    tabs: &'a [SliceScopeId],
    hooks: &'a [SliceScopeId],
    key_hooks: &'a [SliceScopeId],
) -> Vec<Scope> {
    match row.site {
        BindSite::OwnScope => vec![Scope::Dynamic(row.scope.clone())],
        BindSite::StaticScopes(names) => names
            .iter()
            .filter_map(|name| match name.parse::<Scope>() {
                Ok(scope) => Some(scope),
                Err(()) => {
                    tracing::warn!(
                        route = row.route_id.as_str(),
                        scope = name,
                        "static-scope row names an unknown scope; key unbound there"
                    );
                    None
                }
            })
            .collect(),
        BindSite::GlobalToggle => {
            let mut scopes: Vec<Scope> = [Scope::Normal, Scope::Input].into_iter().collect();
            // A picker scope is a modal overlay: while one is open the global
            // toggles still have to work. Modal scopes are discovered from the
            // routes themselves — a slice declares its picker modal — so a new
            // picker needs no line here.
            for scope in routes.modal_scopes() {
                scopes.push(Scope::Dynamic(scope));
            }
            for scope in tabs {
                scopes.push(Scope::Dynamic(scope.clone()));
            }
            for scope in hooks {
                if *scope != row.scope {
                    scopes.push(Scope::Dynamic(scope.clone()));
                }
            }
            // Key-hook scopes are intentionally excluded — this is the
            // GlobalToggle pass, and those scopes carry catch-all key
            // hooks instead (capture mode hermeticity). Modal scopes
            // (declared by their slice) are excluded too: while such a
            // scope is on top, other slices' toggles do not pierce it —
            // its keys come from its own rows and hooks. Both include
            // scopes that host their own rows (term:control hosts the
            // release-control row); the owning slice still binds there.
            scopes.retain(|scope| match scope {
                Scope::Dynamic(id) => {
                    // The row's own scope always binds (the owning
                    // slice's rows are the point).
                    if id == &row.scope {
                        return true;
                    }
                    !key_hooks.contains(id) && !routes.is_modal_scope(id)
                }
                _ => true,
            });
            scopes
        }
    }
}

/// Collects every dynamic scope the route table knows about: row scopes
/// (tab scopes) plus hook scopes (input and key-hook scopes). Used to
/// spread per-scope composition chrome (the `<M-t>` toggle) across
/// slices.
#[must_use]
pub fn dynamic_scopes(routes: &KeyRoutes) -> Vec<SliceScopeId> {
    let mut scopes: Vec<SliceScopeId> = routes.rows().iter().map(|r| r.scope.clone()).collect();
    for hook in routes
        .input_hook_scopes()
        .iter()
        .chain(routes.key_hook_scopes().iter())
    {
        if !scopes.contains(hook) {
            scopes.push(hook.clone());
        }
    }
    scopes
}

/// Derives which-key group descriptions from row keys.
///
/// A multi-token sequence (e.g. `gdc` — three keys) implies a group at
/// each proper prefix (`g`, `gd`): the prefix must describe itself or
/// the which-key popup shows it as an undescribed node. Descriptions
/// come from the owning slice's `feature` label. Existing descriptions
/// win: the keymap only fills `"..."` placeholders, so hardcoded group
/// descriptions (`g` → "general") are never clobbered — and the same
/// prefix reached via two slices merges into one group.
///
/// Groups derive at keymap level (not per scope): a scoped leaf binding
/// shadows the shared branch description in its own scope, while scopes
/// without a scoped leaf keep the group visible.
fn derive_groups_from_rows(
    rows: &[RouteRow],
    keymap: &mut Keymap<KeyEvent, Scope, KernelIntent, KeyCategory>,
) {
    let mut prefixes: Vec<(String, &'static str)> = Vec::new();
    for row in rows {
        // The leader placeholder only matters for `<leader>` notation,
        // which row keys never use.
        let tokens = parse_key_sequence::<KeyEvent>(row.key, &plain_key('\\'));
        for n in 1..tokens.len() {
            // A prefix is only derivable when its display form re-parses
            // to exactly the same tokens: plain chars and `<c-x>`/`<m-x>`
            // forms round-trip; named keys (`Tab`, `Esc`) and shifted
            // forms (`S-x`) do not. Joining can also fuse tokens
            // (`<M-a>` + `b` → `<M-ab>`), so equality is checked on the
            // re-parsed sequence, not per token.
            let Some(prefix) = tokens.get(..n) else {
                break;
            };
            let notation = describe_prefix(prefix);
            let reparsed = parse_key_sequence::<KeyEvent>(&notation, &plain_key('\\'));
            if reparsed != prefix {
                break;
            }
            if let Some(existing) = prefixes.iter_mut().find(|(p, _)| *p == notation) {
                if existing.1 != row.feature {
                    existing.1 = "actions";
                }
            } else {
                prefixes.push((notation, row.feature));
            }
        }
    }
    for (prefix, label) in prefixes {
        keymap.describe_group(&prefix, label);
    }
}

/// A bare character key with no modifiers.
fn plain_key(c: char) -> KeyEvent {
    KeyEvent {
        key: Key::Char(c),
        modifiers: jinn_domain::Modifiers::none(),
    }
}

/// Joins parsed key tokens back into display notation.
fn describe_prefix(tokens: &[KeyEvent]) -> String {
    let mut out = String::new();
    for token in tokens {
        out.push_str(&ratatui_which_key::Key::display(token));
    }
    out
}

/// Materializes every attached route row as keymap bindings.
///
/// Called once after all slice activations, before the event loop. The
/// keymap is mutated in place so the generated bindings land in the
/// same tree as the built-in scope bindings.
pub fn bind_route_rows(
    routes: &KeyRoutes,
    keymap: &mut Keymap<KeyEvent, Scope, KernelIntent, KeyCategory>,
) {
    let rows = routes.rows();
    let input_hooks = routes.input_hook_scopes();
    let key_hooks = routes.key_hook_scopes();
    derive_groups_from_rows(&rows, keymap);
    // Row scopes that host other slices' global toggles: every registered
    // scope (rows + input hooks) except the row's own, where its OwnScope
    // rows must win. Key-hook scopes are excluded: nothing from the row
    // spread may land there (capture hermeticity).
    let mut tabs: Vec<SliceScopeId> = rows.iter().map(|r| r.scope.clone()).collect();
    for hook in &input_hooks {
        if !tabs.contains(hook) {
            tabs.push(hook.clone());
        }
    }
    tabs.dedup();
    for row in &rows {
        let category = category(row.category);
        let scopes = scopes_for_row(routes, row, &tabs, &input_hooks, &key_hooks);
        match &row.outcome {
            RouteOutcome::StaticIntent(_) => {
                let Some(intent) = static_intent(row.route_id.as_str()) else {
                    tracing::warn!(
                        route = row.route_id.as_str(),
                        "static route row has no composition intent mapping; key unbound"
                    );
                    continue;
                };
                for scope in scopes {
                    keymap.bind(row.key, intent.clone(), category, scope);
                }
            }
            RouteOutcome::Action {
                action, display, ..
            } => {
                let intent = KernelIntent::Dynamic(jinn_slices::DynamicIntent::new(
                    row.scope.clone(),
                    action,
                    display,
                ));
                for scope in scopes {
                    keymap.bind(row.key, intent.clone(), category, scope);
                }
            }
        }
    }
    // Typing carve-out: a slice with a registered *input* hook captures
    // printable keystrokes in its own scope. The keymap synthesizes the
    // generic editing intents — the char catch-all for printable keys,
    // plus trunk-parity explicit binds for the six non-char editing
    // keys (Backspace used to fall into the catch-all, resolve to
    // nothing, and die in the which-key popup). The intent handler's
    // hook consult (not a god-match arm) routes them to the slice's
    // sync writer via `as_edit_intent`. Key-hook scopes are excluded:
    // their catch-all encodes keys for the slice's own consumer.
    for hook in input_hooks {
        keymap.scope(Scope::Dynamic(hook.clone()), |b| {
            b.bind(
                "<backspace>",
                KernelIntent::DeleteGrapheme,
                KeyCategory::Input,
            )
            .bind(
                "<delete>",
                KernelIntent::DeleteGraphemeForward,
                KeyCategory::Input,
            )
            .bind("<left>", KernelIntent::MoveCursorLeft, KeyCategory::Input)
            .bind("<right>", KernelIntent::MoveCursorRight, KeyCategory::Input)
            .bind(
                "<home>",
                KernelIntent::MoveCursorToStart,
                KeyCategory::Input,
            )
            .bind("<end>", KernelIntent::MoveCursorToEnd, KeyCategory::Input)
            .catch_all(|key: KeyEvent| {
                if let KeyEvent {
                    key: Key::Char(c), ..
                } = &key
                {
                    Some(KernelIntent::InsertChar { ch: *c })
                } else {
                    None
                }
            });
        });
    }
    // Key-hook catch-alls: a slice key hook captures *every* unbound key
    // in its scope (terminal capture mode forwards them to the pty).
    // Bindings beat catch-alls, so rows bound in the scope (the toggle
    // handback) keep priority; there is no global or chrome spread into
    // key-hook scopes, so the hook is the only exit — capture hermetic.
    for hook in key_hooks {
        let Some(hook_fn) = routes.key_hook(&hook) else {
            continue;
        };
        keymap.scope(Scope::Dynamic(hook.clone()), move |b| {
            b.catch_all(move |key: KeyEvent| hook_fn(&key).map(KernelIntent::Dynamic));
        });
    }
}
