//! Chat-input badge rendering via sync plugin hooks.
//!
//! Each render frame, the input-area renderer queries every loaded plugin via
//! the `on_chat_input_badges_render` hook. Plugins return declarative
//! [`BadgeDirective`]s (slot + text + optional style); Rust owns the slot
//! layout and a constrained style vocabulary and draws into a consistent
//! badge location.
//!
//! This is the render-thread direct path (`PluginSyncHooks`, non-`Send`),
//! distinct from the actor/async paths. Hook failures or malformed returns
//! are silently dropped with a `warn!` log so a buggy plugin cannot stall the
//! render loop.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use jinn_domain::{BadgeDirective, RenderCtx, call_hooks_typed};

/// Render hook name for chat-input badges.
const HOOK: &str = "on_chat_input_badges_render";

/// Renders chat-input badges from plugins into the input-area badge slots.
///
/// `input_area` is the chat input box rect. Badges draw into a small strip
/// immediately to the left of (or above) the input box, in a consistent
/// location across all plugins contributing directives.
pub(super) fn render_badges(frame: &mut Frame<'_>, input_area: Rect, ctx: &RenderCtx) {
    // No plugins → nothing to do (skip building the ctx entirely).
    let Some(plugins) = ctx.plugins else {
        return;
    };

    // The active session is what the user is currently looking at.
    let sid = ctx.state.session.active_session_id().clone();
    let badge_ctx = serde_json::json!({ "active_session_id": sid.to_string() });

    // Typed loop: each plugin contributes zero or more directives. Malformed
    // returns are silently dropped (see `call_hooks_typed`).
    let directives = call_hooks_typed::<BadgeDirective>(plugins, HOOK, &badge_ctx);

    draw_directives(frame.buffer_mut(), input_area, &directives);
}

/// Draws the directives into the badge strip above the input box.
///
/// v1 layout: a single row immediately above the input box. Each directive
/// is rendered left-to-right with a one-space separator. Slot order within a
/// plugin's contribution is preserved; plugin order follows the load order.
fn draw_directives(buf: &mut Buffer, input_area: Rect, directives: &[BadgeDirective]) {
    if directives.is_empty() {
        return;
    }

    // The badge row sits one line above the input box (clamped to the frame).
    let Some(row_y) = input_area.y.checked_sub(1) else {
        return;
    };

    let mut x = input_area.x;
    for d in directives {
        let style = style_from_name(d.style.as_deref());
        for ch in d.text.chars() {
            if x >= input_area.x + input_area.width {
                break;
            }
            if let Some(cell) = buf.cell_mut((x, row_y)) {
                cell.set_symbol(&ch.to_string());
                cell.set_style(style);
            }
            x += 1;
        }
        // one-space separator between badges
        if let Some(cell) = buf.cell_mut((x, row_y)) {
            cell.set_symbol(" ");
        }
        x += 1;
    }
}

/// Maps the constrained style vocabulary (strings from Lua) to ratatui styles.
fn style_from_name(name: Option<&str>) -> Style {
    match name {
        Some("yellow") => Style::default().fg(Color::Yellow),
        Some("cyan") => Style::default().fg(Color::Cyan),
        Some("green") => Style::default().fg(Color::Green),
        Some("red") => Style::default().fg(Color::Red),
        Some("bold") => Style::default().add_modifier(ratatui::style::Modifier::BOLD),
        _ => Style::default(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use jinn_domain::PluginSyncHooks;
    use serde_json::Value;

    /// Stub backend returning a fixed list of JSON values.
    struct Stub {
        values: Vec<Value>,
    }

    impl PluginSyncHooks for Stub {
        fn call_hooks(&self, _hook: &str, _ctx: &Value) -> Vec<Value> {
            self.values.clone()
        }
    }

    fn buffer(w: u16, h: u16) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        Buffer::empty(area)
    }

    #[test]
    fn empty_directives_draws_nothing() {
        let mut buf = buffer(20, 3);
        let input_area = Rect::new(0, 2, 20, 1);
        draw_directives(&mut buf, input_area, &[]);
        // Row above input (y=1) should remain blank.
        for x in 0..20 {
            let sym = buf.cell((x, 1)).expect("cell").symbol();
            assert!(sym.is_empty() || sym == " ", "x={x} got {sym:?}");
        }
    }

    #[test]
    fn badge_directives_draw_left_to_right_with_separator() {
        let mut buf = buffer(20, 3);
        let input_area = Rect::new(0, 2, 20, 1);
        let directives = vec![
            BadgeDirective {
                slot: "input_badge".to_owned(),
                text: "E".to_owned(),
                style: Some("yellow".to_owned()),
            },
            BadgeDirective {
                slot: "input_spinner".to_owned(),
                text: "✨".to_owned(),
                style: None,
            },
        ];
        draw_directives(&mut buf, input_area, &directives);
        // Row y=1 should contain "E ✨ " starting at x=0.
        assert_eq!(buf.cell((0, 1)).expect("cell").symbol(), "E");
        assert_eq!(buf.cell((1, 1)).expect("cell").symbol(), " ");
        assert_eq!(buf.cell((2, 1)).expect("cell").symbol(), "✨");
    }

    #[test]
    fn style_from_name_maps_constrained_vocabulary() {
        assert_eq!(
            style_from_name(Some("yellow")),
            Style::default().fg(Color::Yellow)
        );
        assert_eq!(style_from_name(Some("unknown")), Style::default());
        assert_eq!(style_from_name(None), Style::default());
    }
}
