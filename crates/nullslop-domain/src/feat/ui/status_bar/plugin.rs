//! Plugin slot registry for the status bar.
//!
//! Plugins can register named "slots" that appear in the status bar.
//! Each slot has a stable ID, section (left/right), priority, and display text.
//! The registry supports upsert (insert or update) and bulk clear operations.

use nullslop_plugin::PluginId;
use uuid::Uuid;

/// Which side of the status bar a plugin slot appears on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotSection {
    /// Left side of the status bar.
    Left,
    /// Right side of the status bar.
    Right,
}

/// A single plugin-owned slot in the status bar.
///
/// Plugins create slots via the `host_status_bar_add_slot` host API.
/// Each slot is identified by `(plugin_id, stable_id)` — upserting with
/// the same pair replaces the existing slot.
#[derive(Debug, Clone)]
pub struct PluginSlot {
    /// The plugin that owns this slot.
    pub plugin_id: PluginId,
    /// Unique ID for this slot instance (used for dedup in routing).
    pub slot_id: Uuid,
    /// Stable identifier provided by the plugin (e.g., "turn-count").
    pub stable_id: String,
    /// Which side of the status bar this slot appears on.
    pub section: SlotSection,
    /// Ordering within section (lower = first).
    pub priority: u32,
    /// The current text to display.
    pub text: String,
}

/// Registry of all plugin status bar slots.
///
/// Provides upsert, clear, and query operations. The registry is stored
/// in [`AppState`](crate::common::app_state::AppState) and mutated by the
/// plugin actor when plugins add/update/clear slots.
#[derive(Debug, Clone, Default)]
pub struct PluginSlotRegistry {
    slots: Vec<PluginSlot>,
}

impl PluginSlotRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or updates a slot identified by `(plugin_id, stable_id)`.
    ///
    /// If a slot with the same plugin and stable ID already exists, it is
    /// replaced. Otherwise, a new slot is appended.
    pub fn upsert(&mut self, slot: PluginSlot) {
        if let Some(existing) = self
            .slots
            .iter_mut()
            .find(|s| s.plugin_id == slot.plugin_id && s.stable_id == slot.stable_id)
        {
            *existing = slot;
        } else {
            self.slots.push(slot);
        }
    }

    /// Removes all slots owned by the given plugin.
    pub fn clear_for_plugin(&mut self, plugin_id: &PluginId) {
        self.slots.retain(|s| s.plugin_id != *plugin_id);
    }

    /// Removes all slots.
    pub fn clear(&mut self) {
        self.slots.clear();
    }

    /// Returns slots for the given section, sorted by priority (ascending).
    pub fn slots_for_section(&self, section: SlotSection) -> Vec<&PluginSlot> {
        let mut matching: Vec<_> = self.slots.iter().filter(|s| s.section == section).collect();
        matching.sort_by_key(|s| s.priority);
        matching
    }

    /// Returns a mutable reference to the internal slot list.
    pub fn slots_mut(&mut self) -> &mut Vec<PluginSlot> {
        &mut self.slots
    }

    /// Updates the text of a slot identified by `(plugin_id, stable_id)`.
    ///
    /// Returns `true` if the slot was found and updated.
    pub fn update_slot_text(&mut self, plugin_id: &PluginId, stable_id: &str, text: &str) -> bool {
        if let Some(slot) = self
            .slots
            .iter_mut()
            .find(|s| s.plugin_id == *plugin_id && s.stable_id == stable_id)
        {
            text.clone_into(&mut slot.text);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin_id(name: &str) -> PluginId {
        PluginId::new(name)
    }

    fn make_slot(plugin: &str, stable_id: &str, section: SlotSection, priority: u32) -> PluginSlot {
        PluginSlot {
            plugin_id: plugin_id(plugin),
            slot_id: Uuid::new_v4(),
            stable_id: stable_id.to_owned(),
            section,
            priority,
            text: format!("{stable_id}-text"),
        }
    }

    #[rstest::rstest]
    fn upsert_adds_new_slot() {
        // Given an empty registry.
        let mut registry = PluginSlotRegistry::new();

        // When upserting a slot.
        let slot = make_slot("my-plugin", "count", SlotSection::Left, 0);
        registry.upsert(slot);

        // Then the registry has one slot.
        assert_eq!(registry.slots_for_section(SlotSection::Left).len(), 1);
    }

    #[rstest::rstest]
    fn upsert_replaces_existing_slot() {
        // Given a registry with one slot.
        let mut registry = PluginSlotRegistry::new();
        registry.upsert(make_slot("p", "id", SlotSection::Left, 0));

        // When upserting with the same plugin_id and stable_id.
        let updated = PluginSlot {
            plugin_id: plugin_id("p"),
            slot_id: Uuid::new_v4(),
            stable_id: "id".to_owned(),
            section: SlotSection::Right,
            priority: 10,
            text: "updated".to_owned(),
        };
        registry.upsert(updated);

        // Then there is still one slot, but with updated values.
        let slots = registry.slots_for_section(SlotSection::Right);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].text, "updated");
        assert_eq!(slots[0].priority, 10);
    }

    #[rstest::rstest]
    fn clear_for_plugin_removes_only_that_plugins_slots() {
        // Given a registry with slots from two plugins.
        let mut registry = PluginSlotRegistry::new();
        registry.upsert(make_slot("a", "x", SlotSection::Left, 0));
        registry.upsert(make_slot("b", "y", SlotSection::Left, 0));

        // When clearing slots for plugin "a".
        registry.clear_for_plugin(&plugin_id("a"));

        // Then only plugin "b" slots remain.
        assert_eq!(registry.slots_for_section(SlotSection::Left).len(), 1);
        assert_eq!(
            registry.slots_for_section(SlotSection::Left)[0].plugin_id,
            plugin_id("b")
        );
    }

    #[rstest::rstest]
    fn clear_removes_all_slots() {
        // Given a registry with multiple slots.
        let mut registry = PluginSlotRegistry::new();
        registry.upsert(make_slot("a", "x", SlotSection::Left, 0));
        registry.upsert(make_slot("b", "y", SlotSection::Right, 0));

        // When clearing.
        registry.clear();

        // Then the registry is empty.
        assert_eq!(registry.slots_for_section(SlotSection::Left).len(), 0);
        assert_eq!(registry.slots_for_section(SlotSection::Right).len(), 0);
    }

    #[rstest::rstest]
    fn slots_for_section_returns_sorted_by_priority() {
        // Given a registry with unsorted slots.
        let mut registry = PluginSlotRegistry::new();
        registry.upsert(make_slot("p", "c", SlotSection::Left, 30));
        registry.upsert(make_slot("p", "a", SlotSection::Left, 10));
        registry.upsert(make_slot("p", "b", SlotSection::Left, 20));

        // When querying slots for the left section.
        let slots = registry.slots_for_section(SlotSection::Left);

        // Then they are sorted by priority.
        assert_eq!(slots[0].stable_id, "a");
        assert_eq!(slots[1].stable_id, "b");
        assert_eq!(slots[2].stable_id, "c");
    }

    #[rstest::rstest]
    fn slots_for_section_filters_by_section() {
        // Given a registry with left and right slots.
        let mut registry = PluginSlotRegistry::new();
        registry.upsert(make_slot("p", "left", SlotSection::Left, 0));
        registry.upsert(make_slot("p", "right", SlotSection::Right, 0));

        // When querying left slots.
        let left = registry.slots_for_section(SlotSection::Left);

        // Then only left slots are returned.
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].stable_id, "left");
    }

    #[rstest::rstest]
    fn update_slot_text_updates_existing_slot() {
        // Given a registry with a slot.
        let mut registry = PluginSlotRegistry::new();
        registry.upsert(make_slot("p", "counter", SlotSection::Left, 0));

        // When updating the slot text.
        let found = registry.update_slot_text(&plugin_id("p"), "counter", "turns: 42");

        // Then the update succeeded and text changed.
        assert!(found);
        assert_eq!(
            registry.slots_for_section(SlotSection::Left)[0].text,
            "turns: 42"
        );
    }

    #[rstest::rstest]
    fn update_slot_text_returns_false_for_missing_slot() {
        // Given an empty registry.
        let mut registry = PluginSlotRegistry::new();

        // When updating a nonexistent slot.
        let found = registry.update_slot_text(&plugin_id("p"), "missing", "text");

        // Then it returns false.
        assert!(!found);
    }
}
