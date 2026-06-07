//! Cache for rendered skill-preview lines.
//!
//! The skill picker re-renders the selected skill's full markdown body — including
//! tree-sitter syntax highlighting — on every render frame. That is expensive
//! enough to noticeably slow down rendering. This cache stores the already-rendered
//! `Line` vectors so repeated frames (and back-and-forth navigation between skills)
//! skip the markdown render entirely.
//!
//! Mirrors the shape of [`SessionPreviewCache`] but keys only on `(skill_name,
//! content_width)` because a skill body is immutable between rescans.
//!
//! Cache invalidation:
//! - **Rescan** (`SkillsScanActor` processing `ScanSkills`): bodies may have
//!   changed on disk → cleared so stale markdown is never redisplayed.
//! - **Theme change** (`FrontendCaches::invalidate_all`): rendered lines embed
//!   theme colors → cleared.
//! - **Picker open/close**: cache is preserved so the user does not pay a
//!   re-render cost when reopening the picker.
//!
//! [`SessionPreviewCache`]: crate::feat::ui::sidebar::sessions::preview::SessionPreviewCache

use parking_lot::Mutex;
use std::collections::HashMap;

use jinn_selection_widget::PreviewCache;
use ratatui::text::Line;

/// Cache for skill-preview rendered lines.
///
/// Keyed by `(skill_name, content_width)` so that:
/// - Switching skills produces a cache miss (different skill name).
/// - Terminal resize produces a cache miss (different width).
///
/// Interior mutability ([`parking_lot::Mutex`]) is used because the [`PreviewCache`] trait
/// methods take `&self` — the cache is borrowed immutably (`Option<&dyn PreviewCache>`)
/// as it is threaded through the widget's render pipeline. The standalone `clear` method
/// takes `&mut self` (matching `SessionPreviewCache`), so `invalidate_all` and the
/// `SkillsScanActor` acquire a write lock for clarity and consistency.
///
/// [`FrontendCaches`]: crate::feat::ui::frontend_state::FrontendCaches
#[derive(Debug, Default)]
pub struct SkillPreviewCache {
    entries: Mutex<HashMap<(String, usize), Vec<Line<'static>>>>,
}

impl SkillPreviewCache {
    /// Creates a new empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears all cached preview lines.
    ///
    /// Called when the active theme changes (via `FrontendCaches::invalidate_all`)
    /// and when skills are rescanned (via `reload_skill_picker_entries`).
    pub fn clear(&mut self) {
        self.entries.lock().clear();
    }

    /// Returns the number of cached entries (for testing).
    pub fn len(&self) -> usize {
        self.entries.lock().len()
    }

    /// Returns the set of skill names currently cached (for testing).
    ///
    /// Returns names across all widths; width is intentionally excluded so callers can verify
    /// *which* skills are cached without knowing the rendered width.
    pub fn skill_names(&self) -> Vec<String> {
        self.entries
            .lock()
            .keys()
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Returns `true` if the cache holds no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.lock().is_empty()
    }
}

impl PreviewCache for SkillPreviewCache {
    fn get(&self, key: &str, width: usize) -> Option<Vec<Line<'static>>> {
        self.entries.lock().get(&(key.to_owned(), width)).cloned()
    }

    fn insert(&self, key: String, width: usize, lines: Vec<Line<'static>>) {
        self.entries.lock().insert((key, width), lines);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jinn_selection_widget::PreviewCache;
    use ratatui::text::Line;

    fn line(s: &str) -> Line<'static> {
        Line::from(s.to_owned())
    }

    #[test]
    fn get_on_empty_cache_returns_none() {
        let cache = SkillPreviewCache::new();
        assert!(cache.get("any-skill", 80).is_none());
    }

    #[test]
    fn insert_then_get_returns_stored_lines() {
        let cache = SkillPreviewCache::new();
        cache.insert("bash".to_owned(), 80, vec![line("rendered bash preview")]);
        let got = cache.get("bash", 80).expect("entry should exist");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].spans.len(), 1);
        assert_eq!(got[0].spans[0].content, "rendered bash preview");
    }

    #[test]
    fn width_is_part_of_the_key() {
        let cache = SkillPreviewCache::new();
        cache.insert("rust".to_owned(), 80, vec![line("width 80")]);
        // Same skill, different width -> miss.
        assert!(cache.get("rust", 100).is_none());
        // Insert at the new width.
        cache.insert("rust".to_owned(), 100, vec![line("width 100")]);
        // Both widths now hit.
        assert!(cache.get("rust", 80).is_some());
        assert!(cache.get("rust", 100).is_some());
    }

    #[test]
    fn different_skills_are_independent() {
        let cache = SkillPreviewCache::new();
        cache.insert("alpha".to_owned(), 80, vec![line("a")]);
        // beta is not cached.
        assert!(cache.get("beta", 80).is_none());
    }

    #[test]
    fn clear_empties_all_entries() {
        let mut cache = SkillPreviewCache::new();
        cache.insert("a".to_owned(), 80, vec![line("a")]);
        cache.insert("b".to_owned(), 100, vec![line("b")]);
        assert_eq!(cache.len(), 2);
        cache.clear();
        assert!(cache.is_empty());
        assert!(cache.get("a", 80).is_none());
        assert!(cache.get("b", 100).is_none());
    }

    #[test]
    fn get_returns_an_owned_clone_not_a_reference() {
        // The PreviewCache trait returns owned Vec<Line>, so callers can hold
        // the result across the cache being mutated.
        let mut cache = SkillPreviewCache::new();
        cache.insert("k".to_owned(), 80, vec![line("v")]);
        let got = cache.get("k", 80).expect("entry should exist");
        cache.clear();
        // The clone survives the clear.
        assert_eq!(got.len(), 1);
    }
}
