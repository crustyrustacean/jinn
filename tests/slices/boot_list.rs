//! The boot list's structural invariants.
//!
//! These are source-level tests on `src/bootstrap/slices.rs`. The thing
//! they guard is a *missing line in a file no behavioural test executes* —
//! the reasoning-effort picker shipped broken exactly that way, wired into
//! the test harness but not into production. A behavioural test would need
//! a full app boot; reading the boot list does not, and holds even while
//! the tree does not compile.

/// The producers block is written before the independents block.
///
/// Value-providing slices must precede the slices that read what they
/// provide. This asserts the grouping survives edits, not just that
/// someone remembered it once.
#[rstest::rstest]
fn producers_are_grouped_before_the_independents() {
    // Given the production boot list.
    let list = boot_list();

    // When the two block markers are located.
    let producers = list
        .find("Block 1: producers")
        .expect("the boot list declares a producers block");
    let independents = list
        .find("Block 2:")
        .expect("the boot list declares an independents block");

    // Then the producers come first.
    assert!(
        producers < independents,
        "producers must precede the slices that read what they provide"
    );
}

/// Each producer names the cell it provides and who consumes it.
///
/// A producer whose comment is gone is a producer whose contract is
/// invisible; the next reader cannot tell what broke if it stops running.
#[rstest::rstest]
#[case("provider_state_slot", "jinn_provider_selection::activate(")]
#[case("token_cache_slot", "jinn_token_count::activate(")]
#[case("session_picker_slot", "jinn_session_store::activate(")]
fn each_producer_documents_the_cell_it_provides(#[case] slot: &str, #[case] producer: &str) {
    // Given the production boot list.
    let list = boot_list();

    // When the producer's activation is located.
    let call = activation_line_containing(&list, producer);

    // Then the nearest comment block above it names what it provides.
    // The activation is preceded by the `let` bindings of its arguments,
    // so the walk passes over those to reach the comment.
    let preceding = &list[..list.find(&call).expect("activation is in the list")];
    let window = preceding
        .lines()
        .rev()
        .take_while(|l| {
            let t = l.trim();
            t.is_empty() || t.starts_with("//") || t.starts_with("let ")
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        window.contains("provides:") && window.contains(slot),
        "the producer of {slot} must carry a comment naming what it provides"
    );
}

/// The producers that the boot list resolves by slot key are all present.
///
/// A producer whose cell is never read is dead wiring; one whose reader
/// is missing is a consumer that will panic at boot.
#[rstest::rstest]
fn producer_cells_are_resolved_by_slot_key() {
    // Given the production boot list and the ctx it builds against.
    let list = boot_list();
    let ctx =
        std::fs::read_to_string("src/bootstrap/ctx.rs").expect("the boot ctx is in every checkout");

    // Then each of the three producer slots is resolved somewhere.
    for slot in [
        "jinn_provider_selection_msg::provider_state_slot()",
        "jinn_session_store_msg::session_picker_slot()",
    ] {
        assert!(
            list.contains(slot),
            "{slot} is never resolved: a consumer of that cell will not find it"
        );
    }
    // The token cache is read by the session actor's deps, which the ctx
    // builds — so it is resolved there rather than in the list itself.
    assert!(
        ctx.contains("jinn_token_count_msg::token_cache_slot()"),
        "the token cache cell is never resolved: the session actor will accumulate through a fresh cache"
    );
}

/// Every activation the boot list calls actually exists as a call.
///
/// Guards the inverse of the picker test above: a stale entry naming an
/// activation that was renamed or deleted fails to compile loudly, but
/// one that was *dropped* while its comment survived does not.
#[rstest::rstest]
fn every_activation_line_is_a_call() {
    // Given the production boot list.
    let list = boot_list();

    // When each jinn_ line is examined.
    for line in list.lines() {
        let trimmed = line.trim();
        // A continuation line (a slot key, a path) is part of the call
        // above it, not an activation of its own.
        if !trimmed.starts_with("jinn_") || !trimmed.contains("(") {
            continue;
        }
        if trimmed.ends_with("),") || trimmed.ends_with("(") {
            continue;
        }
        let (module, rest) = trimmed
            .split_once("::")
            .expect("an activation line names a crate and a function");
        let func = rest.split('(').next().unwrap_or_default();
        assert!(
            func.starts_with("activate")
                || func.starts_with("install_actors")
                || func.starts_with("install_layout_actors")
                || func.starts_with("register_all_cells"),
            "{module}::{func} is wired but is not an activation entry point"
        );
    }
}

/// The boot list is the only place slice activations are called from.
///
/// Two lists means one of them drifts. The harness composes its own, so
/// the check is scoped to the production source tree.
#[rstest::rstest]
fn slice_activations_are_not_called_outside_the_boot_list() {
    // Given every production source file outside the boot list.
    let mut offenders = Vec::new();
    for path in production_sources() {
        if path.ends_with("bootstrap/slices.rs") {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (n, line) in source.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("jinn_")
                && (trimmed.contains("::activate(") || trimmed.contains("::install_actors("))
            {
                offenders.push(format!("{path}:{}: {}", n + 1, trimmed));
            }
        }
    }

    // Then only documented exceptions call an activation.
    for offender in offenders {
        let allowed = offender.contains("app/builder.rs") || offender.contains("bootstrap/ui.rs");
        assert!(
            allowed,
            "slice activation called outside the boot list: {offender}"
        );
    }
}

/// The cell catalog is the only place a slice cell is registered.
///
/// Four hand-maintained seeding lists used to exist — the boot list, the
/// TUI test app builder, `AppState`'s test seeding, and
/// `Services::new_fake` — and they had already drifted. The failure they
/// produced was silent: a harness that omitted a cell rendered nothing
/// while every assertion stayed green.
///
/// A behavioural test for this needs a full app boot and a seeded
/// registry, which is exactly the thing that was drifting. Reading the
/// sources does not, and holds even while the tree does not compile.
#[rstest::rstest]
fn cell_registration_happens_only_in_the_catalog() {
    // Given every slice-crate source outside the catalog crate.
    let mut offenders = Vec::new();
    for path in slice_sources() {
        if path.contains("jinn-cell-catalog") || is_test_only(&path) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        // Everything from the first `#[cfg(test)]` onward is test code: a
        // unit test that seeds the one cell it asserts on is testing that
        // cell, not maintaining a second production list.
        let production = source
            .split_once("#[cfg(test)]")
            .map_or(source.as_str(), |(before, _)| before);
        for (n, line) in production.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") {
                continue;
            }
            // `register_cell` is the slice-facing verb; `.register(` on a
            // `Slices` is the raw form three slices used to reach for.
            if !(trimmed.contains("register_cell(") || trimmed.contains("slices.register(")) {
                continue;
            }
            offenders.push(format!("{path}:{}: {}", n + 1, trimmed));
        }
    }

    // Then no slice registers a cell outside the catalog.
    assert!(
        offenders.is_empty(),
        "a cell is registered outside the catalog; the catalog must be the only \
         registration path:\n{}",
        offenders.join("\n")
    );
}

/// The catalog registers more than one cell, and says how many.
///
/// A catalog that silently registered a single cell would satisfy the
/// structural test above while reintroducing the exact problem: one list,
/// but an incomplete one.
#[rstest::rstest]
fn the_catalog_is_the_full_list() {
    // Given the catalog's source.
    let source = std::fs::read_to_string("crates/jinn-cell-catalog/src/lib.rs")
        .expect("the catalog is in every checkout");

    // When its entries are counted.
    let entries = source.lines().filter(|l| l.trim() == "register!(").count();

    // Then the count matches the constant the function asserts against.
    let declared = source
        .lines()
        .find_map(|l| l.trim().strip_prefix("const EXPECTED_CELL_COUNT: usize = "))
        .and_then(|l| l.trim_end_matches(';').parse::<usize>().ok())
        .expect("the catalog declares its expected cell count");

    assert_eq!(
        entries, declared,
        "the catalog registers {entries} cells but declares {declared}; the assertion \
         would either always pass or always fire"
    );
    assert!(
        entries > 1,
        "the catalog registers {entries} cell — one list that is still incomplete is the \
         failure this crate exists to end"
    );
}

fn boot_list() -> String {
    std::fs::read_to_string("src/bootstrap/slices.rs").expect("the boot list is in every checkout")
}

/// The single line in `list` that calls `needle`.
fn activation_line_containing(list: &str, needle: &str) -> String {
    list.lines()
        .find(|l| l.contains(needle) && !l.trim_start().starts_with("//"))
        .unwrap_or_else(|| panic!("no activation line calls {needle}"))
        .trim()
        .to_owned()
}

fn production_sources() -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec!["src".to_owned()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path.display().to_string());
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path.display().to_string());
            }
        }
    }
    out.sort();
    out
}

/// A source file that exists only to be compiled under `cfg(test)`.
///
/// The slices declare their test modules two ways: a `#[cfg(test)] mod
/// tests` inline, and a separate `*_tests.rs` pulled in from the crate
/// root. Both are test code. A unit test that seeds the one cell it
/// asserts on is exercising that cell, not maintaining a second
/// production list.
fn is_test_only(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.ends_with("_tests.rs") || name == "tests.rs" || name == "test.rs"
}

/// Every `.rs` file under `crates/slices`, plus the catalog crate.
///
/// The slices are where cell registration used to hide, so the scan
/// covers them; `crates/jinn-slices` is excluded because its three
/// infrastructure slots are registered through a private
/// `get_or_register` path by design, and its resolvers are private so no
/// catalog could mint a second handle to them.
fn slice_sources() -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![
        "crates/slices".to_owned(),
        "crates/jinn-cell-catalog".to_owned(),
    ];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path.display().to_string());
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path.display().to_string());
            }
        }
    }
    out.sort();
    out
}
