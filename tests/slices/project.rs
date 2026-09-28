//! Project-slice composition tests.

use crate::common::{activate_preferences, activate_project};

/// The project-add cell resolves after the project slice activates.
///
/// It used to be minted by `jinn_project::activate`, so this test's
/// subject was the activation. It is now the shared catalog, which
/// registers it before any slice activates — so an assertion that the
/// key merely *exists* would pass with the activation deleted entirely.
/// What is worth asserting is the property a reader depends on: the cell
/// resolves as the type its readers ask for, and holds usable state.
#[rstest::rstest]
#[tokio::test]
async fn project_add_cell_resolves_after_activation() {
    // Given fake services whose registry is seeded by the shared catalog.
    let services = jinn_kernel::Services::new_fake().await;

    // When the project slice activates.
    let mut services = services;
    activate_project(&mut services);

    // Then the project-add cell resolves as the type its readers ask for.
    let cell = services
        .slices
        .reader::<jinn_project_msg::ProjectAddInputState>(&jinn_project::project_add_slot())
        .expect("the catalog registers the project-add cell before the project slice activates");
    // And it is usable, not a placeholder of the wrong shape.
    let mut state = cell.read().clone();
    state.text.set("a".to_owned());
    assert_eq!(state.text.input, "a");
}

/// The preferences slice does not wire the project-add overlay.
///
/// This test guards slice *separation*, not cell presence: the project
/// cell is registered by the catalog regardless of which slice activates,
/// so "the preferences activation must not claim the project scope's
/// overlay" is the property that still discriminates the two. It used to
/// be expressed as "the project cell is absent", which the catalog made
/// true unconditionally and therefore meaningless.
#[rstest::rstest]
#[tokio::test]
async fn preferences_activation_does_not_wire_the_project_add_overlay() {
    // Given fresh fake services without project activation.
    let services = jinn_kernel::Services::new_fake().await;

    // When only the preferences persistence slice activates.
    let mut services = services;
    activate_preferences(&mut services);

    // Then no overlay is registered for the project-add scope.
    assert!(
        services
            .slices
            .overlay(&jinn_project::project_add_scope())
            .is_none(),
        "only the project slice may claim the project-add overlay"
    );
}
