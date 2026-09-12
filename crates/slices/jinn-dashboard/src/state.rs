//! Dashboard state — the tab's cell payload and entry model.

use std::collections::HashMap;

use jinn_core_types::ActorLifecycle;

/// A single actor's display data in the dashboard.
#[derive(Debug, Clone)]
pub struct DashboardEntry {
    /// The actor's display name (also its unique key).
    pub name: String,
    /// A short description of what the actor does.
    pub description: Option<String>,
    /// The actor's current lifecycle phase.
    pub lifecycle: ActorLifecycle,
    /// Free-form third column; the owning feature writes its connection or
    /// resolution status here via `ServiceStatusUpdate`.
    pub status_message: Option<String>,
}

/// Owned by [`DashboardCanvasActor`](crate::canvas_actor::DashboardCanvasActor).
/// The actor owns this field; the renderer resolves a read handle.
#[derive(Debug, Clone, Default)]
pub struct DashboardState {
    /// Actor name → entry data.
    actors: HashMap<String, DashboardEntry>,
    /// Insertion-order keys for stable display.
    order: Vec<String>,
    /// Index of the currently selected actor entry.
    selected_index: usize,
    /// Vertical scroll offset in visual lines.
    scroll_offset: u16,
}

impl DashboardState {
    /// Create an empty dashboard state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the index of the currently selected actor entry.
    #[must_use]
    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    /// Returns the current vertical scroll offset in visual lines.
    #[must_use]
    pub fn scroll_offset(&self) -> u16 {
        self.scroll_offset
    }

    /// Clamps `scroll_offset` so the selected entry is always visible within
    /// a viewport of `viewport_height` rows.
    pub fn clamp_scroll(&mut self, viewport_height: u16) {
        self.scroll_offset = self.clamped_offset(viewport_height);
    }

    /// The scroll offset that keeps the selected entry visible within a
    /// viewport of `viewport_height` rows — the pure read-side form of
    /// [`Self::clamp_scroll`], for renderers that hold only a read
    /// handle and clamp per frame instead of mutating the cell.
    #[must_use]
    pub fn clamped_offset(&self, viewport_height: u16) -> u16 {
        if viewport_height == 0 {
            return self.scroll_offset;
        }
        let count = self.order.len() as u16;
        if count == 0 {
            return 0;
        }
        let offset = {
            let sel = u16::try_from(self.selected_index).unwrap_or(u16::MAX);
            let bottom = self
                .scroll_offset
                .saturating_add(viewport_height)
                .saturating_sub(1);
            match sel {
                s if s < self.scroll_offset => s,
                s if s > bottom => s.saturating_sub(viewport_height).saturating_add(1),
                _ => self.scroll_offset,
            }
        };
        let max_offset = count.saturating_sub(viewport_height);
        offset.min(max_offset)
    }

    /// Returns all tracked actors in insertion order.
    #[must_use]
    pub fn actors(&self) -> Vec<&DashboardEntry> {
        self.order
            .iter()
            .filter_map(|name| self.actors.get(name))
            .collect()
    }

    /// Moves the selection to the next actor entry.
    ///
    /// Clamps at the last entry - does nothing if already at the end.
    pub fn select_next(&mut self) {
        let count = self.order.len();
        if count > 0 && self.selected_index < count - 1 {
            self.selected_index += 1;
        }
    }

    /// Moves the selection to the previous actor entry.
    ///
    /// Clamps at the first entry - does nothing if already at the beginning.
    pub fn select_prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Moves the selection to the first actor entry.
    ///
    /// No-op if there are no actors.
    pub fn select_first(&mut self) {
        if !self.order.is_empty() {
            self.selected_index = 0;
        }
    }

    /// Moves the selection to the last actor entry.
    ///
    /// No-op if there are no actors.
    pub fn select_last(&mut self) {
        if !self.order.is_empty() {
            self.selected_index = self.order.len() - 1;
        }
    }

    /// Record that an actor is in (or has returned to) the startup phase.
    ///
    /// If the actor is new it is appended to the display order. Existing
    /// entries keep their description unless a new one is supplied.
    /// Resets the grid to empty — no actors, selection at top, scroll 0.
    pub fn clear(&mut self) {
        self.actors.clear();
        self.order.clear();
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Record that an actor is in (or has returned to) the startup phase.
    ///
    /// If the actor is new it is appended to the display order. Existing
    /// entries keep their description unless a new one is supplied.
    pub fn mark_starting<S>(&mut self, name: S, description: Option<String>)
    where
        S: AsRef<str>,
    {
        self.upsert(name, description, ActorLifecycle::Starting);
    }

    /// Record that an actor has finished starting and is running.
    pub fn mark_running<S>(&mut self, name: S, description: Option<String>)
    where
        S: AsRef<str>,
    {
        self.upsert(name, description, ActorLifecycle::Running);
    }

    /// Record that an actor has shut down (intentionally or via crash).
    pub fn mark_dead<S>(&mut self, name: S, description: Option<String>)
    where
        S: AsRef<str>,
    {
        self.upsert(name, description, ActorLifecycle::Dead);
    }

    /// Update only the free-form status message for an actor, leaving its
    /// lifecycle untouched.
    ///
    /// Creates the entry (as `Starting`) if it does not already exist, so the
    /// gateway can report a connection status before the corresponding
    /// `ActorStarting` bus event arrives.
    pub fn set_status_message<S>(&mut self, name: S, message: Option<String>)
    where
        S: AsRef<str>,
    {
        let name = name.as_ref();
        if !self.actors.contains_key(name) {
            self.order.push(name.to_owned());
            self.actors.insert(
                name.to_owned(),
                DashboardEntry {
                    name: name.to_owned(),
                    description: None,
                    lifecycle: ActorLifecycle::Starting,
                    status_message: message,
                },
            );
            return;
        }
        if let Some(entry) = self.actors.get_mut(name) {
            entry.status_message = message;
        }
    }

    /// Insert-or-update helper applying a new lifecycle and optional
    /// description. Does not touch `status_message` on existing entries.
    fn upsert<S>(&mut self, name: S, description: Option<String>, lifecycle: ActorLifecycle)
    where
        S: AsRef<str>,
    {
        let name = name.as_ref();
        let is_new = !self.actors.contains_key(name);
        if is_new {
            self.order.push(name.to_owned());
            self.actors.insert(
                name.to_owned(),
                DashboardEntry {
                    name: name.to_owned(),
                    description,
                    lifecycle,
                    status_message: None,
                },
            );
            return;
        }
        if let Some(entry) = self.actors.get_mut(name) {
            entry.lifecycle = lifecycle;
            if description.is_some() {
                entry.description = description;
            }
        }
    }
}
