//! Tab domain: tab management types, active tab state, and tab navigation.

use serde::{Deserialize, Serialize};

// --- Active Tab ---

/// The currently active tab in the main area.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActiveTab {
    /// The chat conversation view.
    #[default]
    Chat,
    /// The workflow visualization view.
    Workflow,
}

impl ActiveTab {
    /// All tabs in display order.
    const ALL: [ActiveTab; 2] = [ActiveTab::Chat, ActiveTab::Workflow];

    /// Returns the label shown in the tab bar.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            ActiveTab::Chat => "Chat",
            ActiveTab::Workflow => "Workflow",
        }
    }

    /// Returns all tabs in display order.
    #[must_use]
    pub const fn all() -> &'static [ActiveTab] {
        &Self::ALL
    }

    /// Advance to the next tab, wrapping around.
    #[must_use]
    #[expect(
        clippy::indexing_slicing,
        reason = "modular arithmetic guarantees idx is within bounds of ALL"
    )]
    pub fn next(self) -> Self {
        let idx = self.index();
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    /// Go to the previous tab, wrapping around.
    #[must_use]
    #[expect(
        clippy::indexing_slicing,
        reason = "modular arithmetic guarantees idx is within bounds of ALL"
    )]
    pub fn prev(self) -> Self {
        let idx = self.index();
        let len = Self::ALL.len();
        Self::ALL[(idx + len - 1) % len]
    }

    /// Returns the index of this tab in the display order.
    const fn index(self) -> usize {
        match self {
            ActiveTab::Chat => 0,
            ActiveTab::Workflow => 1,
        }
    }
}

// --- Tab Direction ---

/// Direction for tab cycling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TabDirection {
    /// Move to the next tab (wrapping).
    Next,
    /// Move to the previous tab (wrapping).
    Prev,
}

impl std::fmt::Display for TabDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Next => write!(f, "next"),
            Self::Prev => write!(f, "prev"),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;

    #[rstest::rstest]
    fn next_wraps_from_last_to_first() {
        // Given the last tab.
        // When advancing.
        // Then it wraps to the first tab.
        assert_eq!(ActiveTab::Workflow.next(), ActiveTab::Chat);
    }

    #[rstest::rstest]
    fn prev_wraps_from_first_to_last() {
        // Given the first tab.
        // When going back.
        // Then it wraps to the last tab.
        assert_eq!(ActiveTab::Chat.prev(), ActiveTab::Workflow);
    }

    #[rstest::rstest]
    fn next_then_prev_returns_to_start() {
        // Given any tab.
        for tab in ActiveTab::all() {
            // When advancing then going back.
            // Then we return to the original tab.
            assert_eq!(tab.next().prev(), *tab);
        }
    }

    #[rstest::rstest]
    fn labels_are_distinct() {
        // Given all tabs.
        // When collecting labels from each tab.
        let labels: Vec<&str> = ActiveTab::all().iter().map(ActiveTab::label).collect();

        // Then no two labels are the same.
        for i in 0..labels.len() {
            for j in (i + 1)..labels.len() {
                assert_ne!(labels[i], labels[j]);
            }
        }
    }
}
