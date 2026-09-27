//! The slice-owned draw registry — one draw function per render region.
//!
//! A region is a *place on screen that composition fills*, not a slice.
//! Composition decides where the chat log goes; the chat-log slice
//! decides what is painted there. This registry is the seam between
//! the two: a slice registers [`DrawFn`] for a [`Region`] at
//! activation, and the render pass calls it with the rect it computed.
//!
//! Two properties follow from that split, and both are the point:
//!
//! * **The slice names its own region, not its own element.** A draw
//!   function is registered under a [`Region`] key, so composition
//!   stops string-matching a slice's element name in order to find it.
//! * **An unregistered region paints nothing.** That is the same
//!   degradation `OverlayViews` has always had, and the startup
//!   pairing check is where a missing registration is caught.
//!
//! The registry is generic over the render context, so it can be
//! declared here without naming `jinn-kernel`'s [`RenderCtx`]:
//! `jinn-kernel` must not depend on a slice crate, and this crate
//! depends only on `jinn-theme` and `ratatui`. The kernel instantiates
//! it at its own `RenderCtx`; a slice instantiates it at whatever
//! context its draw function needs.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use parking_lot::RwLock;
use ratatui::Frame;
use ratatui::layout::Rect;

/// A region of the screen a slice has claimed for drawing.
///
/// Composition resolves a region to a rect and asks the registry who
/// draws there. The variants are the regions the chat layout and the
/// full-width tab fill; a slice registers against the one it owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Region {
    /// The scrolling conversation history.
    ChatLog,
    /// The message composer at the bottom of the content area.
    ChatInput,
    /// The one-row busy indicator above the chat bottom line.
    StreamingIndicator,
    /// The two-row status block at the bottom of the main column.
    StatusBar,
    /// The vertical minimap column beside the chat log.
    Minimap,
    /// The sidebar column, right of the chat border.
    Sidebar,
    /// The audit popup overlaying the chat log.
    AuditPopup,
    /// The autocomplete popup anchored to the input box.
    Autocomplete,
}

impl Region {
    /// Every region, in no particular order. Backs a startup check
    /// that reports which regions went unregistered.
    #[must_use]
    pub fn all() -> &'static [Region] {
        &[
            Region::ChatLog,
            Region::ChatInput,
            Region::StreamingIndicator,
            Region::StatusBar,
            Region::Minimap,
            Region::Sidebar,
            Region::AuditPopup,
            Region::Autocomplete,
        ]
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Region::ChatLog => "chat-log",
            Region::ChatInput => "chat-input",
            Region::StreamingIndicator => "streaming-indicator",
            Region::StatusBar => "status-bar",
            Region::Minimap => "minimap",
            Region::Sidebar => "sidebar",
            Region::AuditPopup => "audit-popup",
            Region::Autocomplete => "autocomplete",
        };
        f.write_str(name)
    }
}

/// A slice-registered draw function: paints one region for one frame.
///
/// `rects` is the frame's selectable-region accumulator. A draw
/// function pushes a rect when the region it paints supports mouse
/// selection — this is why the slot is a parameter rather than a
/// field on the context: the accumulator is composition's, and the
/// decision is the slice's.
pub type DrawFn<C> = Arc<dyn Fn(&mut Frame<'_>, Rect, &C, &mut Vec<Rect>) + Send + Sync>;

/// The frame a draw function paints into, plus the render context.
///
/// Wrapping `ratatui::Frame` behind a type alias keeps the draw
/// function's signature readable and gives the trait bound one place
/// to live.
pub type SliceFrame<'f> = ratatui::Frame<'f>;

/// A draw function wrapped for `Debug` (closures are not `Debug`).
#[derive(Clone)]
struct DrawEntry<C: 'static>(DrawFn<C>);

impl<C: 'static> fmt::Debug for DrawEntry<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DrawFn(..)")
    }
}

/// The registry of slice-registered draw functions, keyed by region.
#[derive(Clone, Debug)]
pub struct RenderSlots<C: 'static> {
    slots: Arc<RwLock<HashMap<Region, DrawEntry<C>>>>,
}

impl<C: 'static> Default for RenderSlots<C> {
    fn default() -> Self {
        Self {
            slots: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl<C: 'static> RenderSlots<C> {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the draw function for `region`, replacing any previous
    /// one. Called once per region at slice activation; a slice that
    /// claims no region registers nothing.
    pub fn register(&self, region: Region, draw: DrawFn<C>) {
        self.slots.write().insert(region, DrawEntry(draw));
    }

    /// Returns the draw function registered for `region`, if any.
    #[must_use]
    pub fn draw(&self, region: Region) -> Option<DrawFn<C>> {
        self.slots.read().get(&region).map(|entry| entry.0.clone())
    }

    /// The regions that have a draw function, sorted for stable display.
    #[must_use]
    pub fn registered(&self) -> Vec<Region> {
        let mut regions: Vec<Region> = self.slots.read().keys().copied().collect();
        regions.sort();
        regions
    }
}

#[cfg(test)]
mod tests {
    use super::Region;
    use super::RenderSlots;

    #[rstest::rstest]
    #[test]
    fn register_then_resolve_roundtrips_region() {
        // Given an empty draw registry.
        let slots: RenderSlots<u8> = RenderSlots::new();

        // When registering a draw function for a region.
        slots.register(
            Region::ChatLog,
            std::sync::Arc::new(|_, _, ctx: &u8, _| {
                let _ = ctx;
            }),
        );

        // Then the registry resolves it back for that region.
        assert!(slots.draw(Region::ChatLog).is_some());
        // And an unregistered region resolves nothing.
        assert!(slots.draw(Region::StatusBar).is_none());
    }

    #[rstest::rstest]
    #[test]
    fn reregister_replaces_the_previous_draw_fn() {
        // Given a registry with one draw function for a region.
        let slots: RenderSlots<u8> = RenderSlots::new();
        slots.register(Region::ChatLog, std::sync::Arc::new(|_, _, _, _| {}));
        slots.register(Region::ChatLog, std::sync::Arc::new(|_, _, _, _| {}));

        // When enumerating the registered regions.
        let registered = slots.registered();

        // Then the region appears once, not twice.
        assert_eq!(registered, vec![Region::ChatLog]);
    }

    #[rstest::rstest]
    #[test]
    fn every_region_has_a_distinct_display_name() {
        // Given every region the layout fills.
        let regions = Region::all();

        // When rendering each to its display name.
        let names: Vec<String> = regions.iter().map(ToString::to_string).collect();

        // Then the names are unique, so a diagnostic naming one region
        // never names two.
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "duplicate region name: {names:?}");
    }

    #[rstest::rstest]
    #[test]
    fn empty_registry_reports_no_registered_regions() {
        // Given an empty draw registry.
        let slots: RenderSlots<u8> = RenderSlots::new();

        // When enumerating the registered regions.
        let registered = slots.registered();

        // Then nothing is registered.
        assert!(registered.is_empty());
    }
}
