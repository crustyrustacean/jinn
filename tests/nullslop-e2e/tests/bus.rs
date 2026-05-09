//! Bus-based integration tests — DISABLED.
//!
//! The bus (`nullslop-component-core`) was deleted in Phase 7. These tests
//! relied on the bus and its handler dispatch. They need to be rewritten
//! to test via the Coordinator/Projector actors instead.
//!
//! See `.plans/simplify/high-level.md` Phase 9 for the rewrite plan.

// All bus test code has been disabled. The file is kept as a placeholder
// so the Cucumber test runner doesn't break. To re-enable, rewrite the
// step definitions to use the Coordinator/Projector actors.

#[cfg(test)]
mod tests {
    #[test]
    fn bus_tests_disabled_pending_rewrite() {
        // Bus was deleted in Phase 7. These tests need rewriting.
        // See Phase 9 in the high-level plan.
    }
}
