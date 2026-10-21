//! Shared render helpers for tests.
//!
//! Rendering a UI element in a test used to mean spelling out the same
//! four-step preamble every time: build a [`Slices`] registry, build an
//! [`OverlayViews`] registry, build a [`RenderCtx`] over both, and draw it
//! into a test terminal. The helpers here collapse that preamble into one
//! call, so a test says what it renders and where, and stops restating
//! the plumbing.
//!
//! Four shapes cover the render paths in the workspace:
//!
//! - [`render_element`] drives a [`UiElement`], whose render signature
//!   takes a [`RenderCtx`] directly.
//! - [`render_element_with_slices`] is the same over a registry the test
//!   seeded itself.
//! - [`render_element_with_config`] is the same over a configuration layer
//!   the test built itself.
//! - [`render_draw`] drives any draw closure over a `&dyn DrawContext`, for
//!   the slice sections and the sidebar container, whose render signature
//!   is the kernel-free one.
//!
//! Every helper returns the test terminal and the rect it drew into, so a
//! test keeps the `terminal.backend().buffer()` reads it already had.
//!
//! [`seed_cell`] covers the other half of the test plumbing: registering a
//! slice's cell so a render path finds something to read.

#![allow(
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::panic,
    reason = "test render helpers: a test terminal and a test draw never fail"
)]

use std::any::Any;

use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

use crate::common::app_state::AppState;
use crate::common::render_ctx::RenderCtx;
use crate::common::ui_element::UiElement;
use jinn_slices::DrawContext;
use jinn_slices::Slices;
use jinn_slices::SlotKey;
use jinn_slices::TypedCell;

/// A test terminal, and the rect it hands to a draw.
pub type TestTerminal = (Terminal<TestBackend>, Rect);

/// Renders `element` into a fresh `width` x `height` terminal.
///
/// The element draws into the whole terminal. A caller that needs the
/// terminal's rect as well uses [`render_element_with_area`]; a caller that
/// wants the element to paint into a sub-rect of a larger terminal uses
/// [`render_element_in`].
#[must_use]
pub fn render_element(
    element: &mut dyn UiElement,
    state: &AppState,
    width: u16,
    height: u16,
) -> Terminal<TestBackend> {
    render_element_in(
        element,
        state,
        &Slices::new(),
        width,
        height,
        Rect::new(0, 0, width, height),
    )
}

/// Renders `element` into a fresh terminal and returns the rect it drew into.
///
/// The same as [`render_element`], for the caller that needs the area as
/// well as the terminal.
#[must_use]
pub fn render_element_with_area(
    element: &mut dyn UiElement,
    state: &AppState,
    width: u16,
    height: u16,
) -> TestTerminal {
    let area = Rect::new(0, 0, width, height);
    (
        render_element_in(element, state, &Slices::new(), width, height, area),
        area,
    )
}

/// Renders `element` into a fresh `width` x `height` terminal over `slices`.
///
/// The same as [`render_element`], for the test that seeded a cell into a
/// registry of its own and needs the render path to read it.
#[must_use]
pub fn render_element_with_slices(
    element: &mut dyn UiElement,
    state: &AppState,
    slices: &Slices,
    width: u16,
    height: u16,
) -> Terminal<TestBackend> {
    render_element_in(
        element,
        state,
        slices,
        width,
        height,
        Rect::new(0, 0, width, height),
    )
}

/// Renders `element` into a fresh `width` x `height` terminal, drawing into `area`.
///
/// The terminal is sized independently of the draw area, which is what a
/// test that paints into a zero-width or zero-height sub-rect needs: the
/// frame stays full-size while the element draws into nothing.
#[must_use]
pub fn render_element_in(
    element: &mut dyn UiElement,
    state: &AppState,
    slices: &Slices,
    width: u16,
    height: u16,
    area: Rect,
) -> Terminal<TestBackend> {
    let overlay_views = jinn_slices::OverlayViews::new();
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("a test terminal always constructs");
    let ctx = RenderCtx::new_with_default_config(state, slices, &overlay_views);
    terminal
        .draw(|frame| element.render(frame, area, &ctx))
        .expect("a test draw never fails");
    terminal
}

/// Renders `element` into a fresh terminal over `config`.
///
/// The same as [`render_element`], for the test that needs a configuration
/// layer it built itself.
#[must_use]
pub fn render_element_with_config(
    element: &mut dyn UiElement,
    state: &AppState,
    slices: &Slices,
    config: &jinn_config::ConfigLayer,
    width: u16,
    height: u16,
) -> Terminal<TestBackend> {
    render_element_in_with_config(
        element,
        state,
        slices,
        config,
        width,
        height,
        Rect::new(0, 0, width, height),
    )
}

/// Renders `element` into a fresh terminal over `config`, returning the rect too.
///
/// The same as [`render_element_with_config`], for the caller that needs the
/// area as well as the terminal.
#[must_use]
pub fn render_element_with_config_and_area(
    element: &mut dyn UiElement,
    state: &AppState,
    slices: &Slices,
    config: &jinn_config::ConfigLayer,
    width: u16,
    height: u16,
) -> TestTerminal {
    let area = Rect::new(0, 0, width, height);
    (
        render_element_in_with_config(element, state, slices, config, width, height, area),
        area,
    )
}

/// Renders `element` into a `width` x `height` terminal over `config`, drawing into `area`.
///
/// The same as [`render_element_in`], for the test that renders over a
/// configuration layer it built itself.
#[must_use]
pub fn render_element_in_with_config(
    element: &mut dyn UiElement,
    state: &AppState,
    slices: &Slices,
    config: &jinn_config::ConfigLayer,
    width: u16,
    height: u16,
    area: Rect,
) -> Terminal<TestBackend> {
    let overlay_views = jinn_slices::OverlayViews::new();
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("a test terminal always constructs");
    let ctx = RenderCtx::new(state, slices, &overlay_views, config);
    terminal
        .draw(|frame| element.render(frame, area, &ctx))
        .expect("a test draw never fails");
    terminal
}

/// Draws `element` again into an existing terminal.
///
/// The test that renders once, changes the state, and renders again keeps
/// one terminal across both frames. This helper redraws into it, so the
/// test's second frame is a call rather than a restated preamble.
pub fn redraw_element(
    element: &mut dyn UiElement,
    state: &AppState,
    slices: &Slices,
    terminal: &mut Terminal<TestBackend>,
    area: Rect,
) {
    let overlay_views = jinn_slices::OverlayViews::new();
    let ctx = RenderCtx::new_with_default_config(state, slices, &overlay_views);
    terminal
        .draw(|frame| element.render(frame, area, &ctx))
        .expect("a test draw never fails");
}

/// Draws `element` again into an existing terminal, over `config`.
///
/// The same as [`redraw_element`], for the test that renders over a
/// configuration layer it built itself.
pub fn redraw_element_with_config(
    element: &mut dyn UiElement,
    state: &AppState,
    slices: &Slices,
    config: &jinn_config::ConfigLayer,
    terminal: &mut Terminal<TestBackend>,
    area: Rect,
) {
    let overlay_views = jinn_slices::OverlayViews::new();
    let ctx = RenderCtx::new(state, slices, &overlay_views, config);
    terminal
        .draw(|frame| element.render(frame, area, &ctx))
        .expect("a test draw never fails");
}

/// Draws into a fresh `width` x `height` test terminal and returns it.
///
/// The draw closure receives the frame and a context over the default
/// (empty) configuration layer, so a test that does not care about
/// configuration does not have to build one.
///
/// The overlay registry is empty: no test in the workspace registers an
/// overlay view before rendering, and a scope with no registered overlay
/// renderer paints nothing.
#[must_use]
pub fn render_draw<F>(state: &AppState, width: u16, height: u16, draw: F) -> Terminal<TestBackend>
where
    F: FnOnce(&mut Frame<'_>, Rect, &dyn DrawContext<AppState>),
{
    render_draw_with_slices(state, &Slices::new(), width, height, draw)
}

/// Draws into a fresh `width` x `height` test terminal over `slices`.
///
/// The same as [`render_draw`], for the test that seeded a cell into a
/// registry of its own and needs the render path to read it.
#[must_use]
pub fn render_draw_with_slices<F>(
    state: &AppState,
    slices: &Slices,
    width: u16,
    height: u16,
    draw: F,
) -> Terminal<TestBackend>
where
    F: FnOnce(&mut Frame<'_>, Rect, &dyn DrawContext<AppState>),
{
    let overlay_views = jinn_slices::OverlayViews::new();
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("a test terminal always constructs");
    let ctx = RenderCtx::new_with_default_config(state, slices, &overlay_views);
    let area = Rect::new(0, 0, width, height);
    terminal
        .draw(|frame| draw(frame, area, &ctx))
        .expect("a test draw never fails");
    terminal
}

/// Draws into a fresh `width` x `height` test terminal over `config`.
///
/// The same as [`render_draw`], for the test that needs a configuration
/// layer it built itself.
#[must_use]
pub fn render_draw_with_config<F>(
    state: &AppState,
    slices: &Slices,
    config: &jinn_config::ConfigLayer,
    width: u16,
    height: u16,
    draw: F,
) -> Terminal<TestBackend>
where
    F: FnOnce(&mut Frame<'_>, Rect, &dyn DrawContext<AppState>),
{
    let overlay_views = jinn_slices::OverlayViews::new();
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("a test terminal always constructs");
    let ctx = RenderCtx::new(state, slices, &overlay_views, config);
    let area = Rect::new(0, 0, width, height);
    terminal
        .draw(|frame| draw(frame, area, &ctx))
        .expect("a test draw never fails");
    terminal
}

/// Draws again into an existing terminal.
///
/// The same as [`render_draw`], for the test that renders once, changes the
/// state, and renders again into the same terminal.
pub fn redraw_draw<F>(state: &AppState, terminal: &mut Terminal<TestBackend>, draw: F)
where
    F: FnOnce(&mut Frame<'_>, Rect, &dyn DrawContext<AppState>),
{
    let overlay_views = jinn_slices::OverlayViews::new();
    let slices = Slices::new();
    let ctx = RenderCtx::new_with_default_config(state, &slices, &overlay_views);
    let size = terminal.size().unwrap_or(ratatui::layout::Size::new(0, 0));
    let area = Rect::new(0, 0, size.width, size.height);
    terminal
        .draw(|frame| draw(frame, area, &ctx))
        .expect("a test draw never fails");
}

/// Draws into a fresh `width` x `height` test terminal with a plain closure.
///
/// For the test that draws something that is not a [`UiElement`] and needs
/// no context at all — a popup drawn straight into the frame, say.
#[must_use]
pub fn render_frame<F>(width: u16, height: u16, draw: F) -> Terminal<TestBackend>
where
    F: FnOnce(&mut Frame<'_>),
{
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("a test terminal always constructs");
    terminal
        .draw(|frame| draw(frame))
        .expect("a test draw never fails");
    terminal
}

/// Hands a context to `f` over the default (empty) configuration layer.
///
/// For the caller that needs a context but does not draw: a section
/// asking for its content height, for instance.
pub fn with_render_ctx<T, F>(state: &AppState, slices: &Slices, f: F) -> T
where
    F: FnOnce(&RenderCtx<'_>) -> T,
{
    let overlay_views = jinn_slices::OverlayViews::new();
    let ctx = RenderCtx::new_with_default_config(state, slices, &overlay_views);
    f(&ctx)
}

/// Hands a context over `config` to `f`.
///
/// The same as [`with_render_ctx`], for the caller whose section reads its
/// own configuration out of the context — a section that counts the servers
/// enabled in the configuration layer, for instance. The default layer is
/// empty, so dropping the caller's layer silently changes what the section
/// sees.
pub fn with_render_ctx_and_config<T, F>(
    state: &AppState,
    slices: &Slices,
    config: &jinn_config::ConfigLayer,
    f: F,
) -> T
where
    F: FnOnce(&RenderCtx<'_>) -> T,
{
    let overlay_views = jinn_slices::OverlayViews::new();
    let ctx = RenderCtx::new(state, slices, &overlay_views, config);
    f(&ctx)
}

/// Registers `payload` under `slot` and returns the cell handle.
///
/// The test seam for "this slice's cell must exist before the render path
/// reads it". A fresh [`Slices`] never has the slot registered, so a
/// failure here means the test seeded the same slot twice — the panic says
/// which slot, so the double-seed is visible.
pub fn seed_cell<T>(slices: &Slices, slot: &SlotKey, payload: T) -> TypedCell<T>
where
    T: Any + Send + Sync,
{
    slices
        .register(slot.clone(), payload)
        .unwrap_or_else(|taken| panic!("cell slot already claimed: {}", taken.key))
}

/// Draws a slice section into a fresh `width` x `height` test terminal.
///
/// A sidebar section renders through the `DrawContext` trait rather than
/// `UiElement`, so it does not go through [`render_element`]. This helper
/// builds the same context and hands it to the caller's draw closure.
///
/// The overlay registry is empty, for the same reason as [`render_element`]:
/// no test registers an overlay view before rendering.
#[must_use]
pub fn render_section<F>(
    state: &AppState,
    slices: &Slices,
    width: u16,
    height: u16,
    draw: F,
) -> Terminal<TestBackend>
where
    F: FnOnce(&mut Frame<'_>, Rect, &dyn DrawContext<AppState>),
{
    let overlay_views = jinn_slices::OverlayViews::new();
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("a test terminal always constructs");
    let ctx = RenderCtx::new_with_default_config(state, slices, &overlay_views);
    let area = Rect::new(0, 0, width, height);
    terminal
        .draw(|frame| draw(frame, area, &ctx))
        .expect("a test draw never fails");
    terminal
}

/// Draws into a fresh `width` x `height` test terminal over `slices`,
/// handing the closure the full [`RenderCtx`].
///
/// The same as [`render_draw_with_slices`], for the test whose draw function
/// takes a `&RenderCtx` rather than a `&dyn DrawContext`.
#[must_use]
pub fn render_draw_with_ctx<F>(
    state: &AppState,
    slices: &Slices,
    width: u16,
    height: u16,
    draw: F,
) -> Terminal<TestBackend>
where
    F: FnOnce(&mut Frame<'_>, Rect, &RenderCtx<'_>),
{
    let overlay_views = jinn_slices::OverlayViews::new();
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("a test terminal always constructs");
    let ctx = RenderCtx::new_with_default_config(state, slices, &overlay_views);
    let area = Rect::new(0, 0, width, height);
    terminal
        .draw(|frame| draw(frame, area, &ctx))
        .expect("a test draw never fails");
    terminal
}

/// Draws a slice section into a fresh terminal, returning the area too.
///
/// The same as [`render_section`], for the caller that needs the rect the
/// section painted into.
#[must_use]
pub fn render_section_with_area<F>(
    state: &AppState,
    width: u16,
    height: u16,
    draw: F,
) -> (Terminal<TestBackend>, Rect)
where
    F: FnOnce(&mut Frame<'_>, Rect, &dyn DrawContext<AppState>),
{
    let overlay_views = jinn_slices::OverlayViews::new();
    let slices = Slices::new();
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("a test terminal always constructs");
    let ctx = RenderCtx::new_with_default_config(state, &slices, &overlay_views);
    let area = Rect::new(0, 0, width, height);
    terminal
        .draw(|frame| draw(frame, area, &ctx))
        .expect("a test draw never fails");
    (terminal, area)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use ratatui::layout::Rect;

    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::ui_element_fake::FakeUiElement;

    /// A slot key no other test in the workspace claims.
    fn test_slot() -> SlotKey {
        SlotKey::builtin("jinn-kernel-test", "render-test-slot")
    }

    #[rstest::rstest]
    fn render_element_paints_the_element_into_the_frame() {
        // Given a fake element and an app state.
        let (mut element, _calls): (FakeUiElement, _) = FakeUiElement::new("test");
        let state = AppState::default();

        // When rendering it.
        let (terminal, area) = render_element_with_area(&mut element, &state, 20, 3);

        // Then the terminal is the size of the requested area.
        assert_eq!(area, Rect::new(0, 0, 20, 3));
        assert_eq!(terminal.backend().buffer().area, Rect::new(0, 0, 20, 3));
    }

    #[rstest::rstest]
    fn render_element_in_draws_into_a_sub_rect_of_a_larger_terminal() {
        // Given a fake element and an app state.
        let (mut element, _calls): (FakeUiElement, _) = FakeUiElement::new("test");
        let state = AppState::default();

        // When rendering it into a zero-height sub-rect of a full-size terminal.
        let collapsed = Rect::new(0, 0, 40, 0);
        let terminal = render_element_in(&mut element, &state, &Slices::new(), 40, 3, collapsed);

        // Then the terminal kept its full size.
        assert_eq!(terminal.backend().buffer().area, Rect::new(0, 0, 40, 3));
    }

    #[rstest::rstest]
    fn seed_cell_registers_the_payload_under_the_slot() {
        // Given a fresh registry.
        let slices = Slices::new();
        let slot = test_slot();

        // When seeding a cell.
        let cell = seed_cell(&slices, &slot, 7_u32);

        // Then the cell holds the payload.
        assert_eq!(*cell.read(), 7);
        // And the registry resolves the slot to that payload type.
        assert!(slices.reader::<u32>(&slot).is_some());
    }

    #[rstest::rstest]
    #[should_panic(expected = "cell slot already claimed")]
    fn seed_cell_on_a_claimed_slot_fails_with_the_slot_named() {
        // Given a registry whose slot is already claimed.
        let slices = Slices::new();
        let slot = test_slot();
        let _cell = seed_cell(&slices, &slot, 7_u32);

        // When seeding the same slot again.
        let _second = seed_cell(&slices, &slot, 8_u32);

        // Then the failure names the contested slot.
    }
}
