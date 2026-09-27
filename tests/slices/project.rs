//! Project-slice composition tests.

use crate::common::{activate_preferences, activate_project};

#[rstest::rstest]
#[tokio::test]
async fn project_activation_registers_project_add_cell() {
    // Given fresh fake services without project activation.
    let services = jinn_kernel::Services::new_fake().await;

    // When the project slice activates.
    let mut services = services;
    activate_project(&mut services);

    // Then the project-add cell is registered.
    assert!(
        services
            .slices
            .slots()
            .contains(&jinn_project::project_add_slot())
    );
}

#[rstest::rstest]
#[tokio::test]
async fn preferences_activation_does_not_register_project_add_cell() {
    // Given fresh fake services without project activation.
    let services = jinn_kernel::Services::new_fake().await;

    // When only the preferences persistence slice activates.
    let mut services = services;
    activate_preferences(&mut services);

    // Then the project-add cell is absent.
    assert!(
        !services
            .slices
            .slots()
            .contains(&jinn_project::project_add_slot())
    );
}
