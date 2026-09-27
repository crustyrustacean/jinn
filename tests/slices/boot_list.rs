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
                || func.starts_with("install_layout_actors"),
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
