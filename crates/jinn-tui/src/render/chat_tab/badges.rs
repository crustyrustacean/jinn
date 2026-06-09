//! Chat-input badge rendering via sync plugin hooks.
//!
//! Each render frame, the input-area renderer queries every loaded plugin via
//! the `on_chat_input_badges_render` hook. Plugins return declarative
//! [`BadgeDirective`]s (slot + styled segments); Rust owns the slot
//! layout and draws into a consistent badge location.
//!
//! Plugins receive the full theme palette via `ctx.theme_styles` and use any
//! theme field name as a style string (e.g. `"streaming"`, `"accent_action"`).
//! Rust resolves those names against a [`HashMap`] built from [`Theme::style_map`].
//!
//! This is the render-thread direct path (`PluginSyncHooks`, non-`Send`),
//! distinct from the actor/async paths. Hook failures or malformed returns
//! are silently dropped with a `warn!` log so a buggy plugin cannot stall the
//! render loop.

use std::collections::HashMap;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

#[cfg(test)]
use jinn_domain::feat::theme::Theme;
use jinn_domain::{BadgeDirective, RenderCtx, call_hooks_typed};

/// Render hook name for chat-input badges.
const HOOK: &str = "on_chat_input_badges_render";

/// Renders chat-input badges from plugins into the input-area badge slots.
/// `input_area` is the chat input box rect. Badges render right-aligned on the
/// input box's bottom border line (same row as the `[QUEUE]`/`[STEER]` mode
/// badge), overlaying the `─` glyphs.
pub(super) fn render_badges(frame: &mut Frame<'_>, input_area: Rect, ctx: &RenderCtx) {
    // No plugins → nothing to do (skip building the ctx entirely).
    let Some(plugins) = ctx.plugins else {
        return;
    };

    // The active session is what the user is currently looking at. `mode` lets a
    // plugin gate its presentation on the current scope (e.g. dim a hotkey legend
    // outside Input mode). The host provides the data; the plugin decides styling.
    let badge_ctx = build_badge_ctx(ctx);

    // Typed loop: each plugin contributes zero or more directives. Malformed
    // returns are silently dropped (see `call_hooks_typed`).
    let directives = call_hooks_typed::<BadgeDirective>(plugins, HOOK, &badge_ctx);

    let style_map = ctx.state.frontend.theme.style_map();
    draw_directives(frame.buffer_mut(), input_area, &directives, &style_map);
}

/// Builds the JSON ctx handed to `on_chat_input_badges_render`.
///
/// `active_session_id` is what the user is currently looking at; `mode` lets a
/// plugin gate its presentation on the current scope (e.g. dim a hotkey legend
/// outside Input mode). The host provides the data; the plugin decides styling.
fn build_badge_ctx(ctx: &RenderCtx) -> serde_json::Value {
    let sid = ctx.state.session.active_session_id().clone();
    let mode = ctx.state.frontend.scope_stack.current().mode();
    serde_json::json!({
        "active_session_id": sid.to_string(),
        "mode": mode.to_string(),
    })
}

/// Draws the directives right-aligned on the input box's bottom border row.
///
/// Each directive contributes one `Line` of styled segments; directives are
/// concatenated left-to-right with a one-space separator. The combined line is
/// right-aligned within `input_area`, drawn on the border row so it overlays
/// the `─` glyphs — matching the `[QUEUE]`/`[STEER]` mode-badge precedent.
fn draw_directives(
    buf: &mut Buffer,
    input_area: Rect,
    directives: &[BadgeDirective],
    style_map: &HashMap<&str, Style>,
) {
    if directives.is_empty() {
        return;
    }

    let Some(line) = build_badge_line(directives, style_map) else {
        return;
    };
    let width = u16::try_from(line.width()).unwrap_or(input_area.width);
    if width == 0 {
        return;
    }

    // Bottom border row; rightmost cell at the input area's right edge.
    let row_y = input_area.bottom().saturating_sub(1);
    let right = input_area.right().saturating_sub(1);
    let start_x = right
        .saturating_sub(width.saturating_sub(1))
        .max(input_area.x);
    let area = Rect {
        x: start_x,
        y: row_y,
        width,
        height: 1,
    };
    Paragraph::new(line).render(area, buf);
}

/// Builds a single styled line from all directives' segments.
///
/// Directives are joined with a one-space separator. Returns `None` if the
/// segments produce no spans.
fn build_badge_line(directives: &[BadgeDirective], style_map: &HashMap<&str, Style>) -> Option<Line<'static>> {
    let mut spans: Vec<Span> = Vec::new();
    for (i, d) in directives.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        spans.extend(d.segments.iter().map(|seg| {
            Span::styled(
                seg.text.clone(),
                style_from_name(seg.style.as_deref(), style_map),
            )
        }));
    }
    if spans.is_empty() {
        None
    } else {
        Some(Line::from(spans))
    }
}

/// Resolves a style name (a theme field name) to a ratatui [`Style`].
///
/// The name is looked up in the theme's style map. Unknown names produce
/// [`Style::default`].
fn style_from_name(name: Option<&str>, style_map: &HashMap<&str, Style>) -> Style {
    name.and_then(|n| style_map.get(n))
        .copied()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use jinn_domain::{AppState, BadgeSegment, FocusScope};

    fn buffer(w: u16, h: u16) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        Buffer::empty(area)
    }

    fn theme() -> Theme {
        jinn_domain::feat::theme::default_theme()
    }

    #[test]
    fn empty_directives_draws_nothing_on_border_row() {
        // Input occupies a single row at y=1; its bottom border row is also y=1.
        let mut buf = buffer(20, 2);
        let input_area = Rect::new(0, 1, 20, 1);
        draw_directives(&mut buf, input_area, &[], &theme().style_map());
        // Then the border row (y=1) stays blank.
        for x in 0..20 {
            let sym = buf.cell((x, 1)).expect("cell").symbol();
            assert!(sym.is_empty() || sym == " ", "x={x} got {sym:?}");
        }
    }

    #[test]
    fn single_badge_is_right_aligned_on_border_row() {
        // Given a 20-wide input area whose bottom border row is y=1.
        let mut buf = buffer(20, 2);
        let input_area = Rect::new(0, 1, 20, 1);
        let directives = vec![BadgeDirective {
            slot: "input_badge".to_owned(),
            segments: vec![BadgeSegment {
                text: "Hi".to_owned(),
                style: None,
            }],
        }];
        // When drawing.
        draw_directives(&mut buf, input_area, &directives, &theme().style_map());
        // Then "Hi" (width 2) sits at the right edge: x=18..19 on row y=1.
        assert_eq!(buf.cell((18, 1)).expect("cell").symbol(), "H");
        assert_eq!(buf.cell((19, 1)).expect("cell").symbol(), "i");
        // And nothing is drawn at the left of the border row.
        assert_eq!(buf.cell((0, 1)).expect("cell").symbol(), " ");
    }

    #[test]
    fn multiple_directives_join_with_one_space_separator() {
        // Given two single-segment badges "A" and "B".
        let mut buf = buffer(20, 2);
        let input_area = Rect::new(0, 1, 20, 1);
        let directives = vec![
            BadgeDirective {
                slot: "input_badge".to_owned(),
                segments: vec![BadgeSegment {
                    text: "A".to_owned(),
                    style: None,
                }],
            },
            BadgeDirective {
                slot: "input_spinner".to_owned(),
                segments: vec![BadgeSegment {
                    text: "B".to_owned(),
                    style: None,
                }],
            },
        ];
        // When drawing.
        draw_directives(&mut buf, input_area, &directives, &theme().style_map());
        // Then the joined "A B" (width 3) is right-aligned: A=17, space=18, B=19.
        assert_eq!(buf.cell((17, 1)).expect("cell").symbol(), "A");
        assert_eq!(buf.cell((18, 1)).expect("cell").symbol(), " ");
        assert_eq!(buf.cell((19, 1)).expect("cell").symbol(), "B");
    }

    #[test]
    fn segments_render_with_per_span_styles() {
        // Given one badge whose two segments carry different style names.
        let mut buf = buffer(20, 2);
        let input_area = Rect::new(0, 1, 20, 1);
        let directives = vec![BadgeDirective {
            slot: "input_badge".to_owned(),
            segments: vec![
                BadgeSegment {
                    text: "[".to_owned(),
                    style: Some("muted_text".to_owned()),
                },
                BadgeSegment {
                    text: "E".to_owned(),
                    style: Some("accent_action".to_owned()),
                },
                BadgeSegment {
                    text: "]".to_owned(),
                    style: Some("muted_text".to_owned()),
                },
            ],
        }];
        let t = theme();
        let sm = t.style_map();
        // When drawing.
        draw_directives(&mut buf, input_area, &directives, &sm);
        // Then each segment's cell carries its resolved style.
        // "[E]" (width 3) right-aligned: [=17, E=18, ]=19.
        assert_eq!(
            buf.cell((17, 1)).expect("cell").style().fg,
            Some(t.muted_text)
        );
        assert_eq!(
            buf.cell((18, 1)).expect("cell").style().fg,
            Some(t.accent_action)
        );
        assert_eq!(
            buf.cell((19, 1)).expect("cell").style().fg,
            Some(t.muted_text)
        );
    }

    #[test]
    fn style_from_name_resolves_theme_fields_and_defaults_unknown() {
        let sm = theme().style_map();
        // Then known theme field names resolve to their fg style.
        assert_eq!(
            style_from_name(Some("accent_action"), &sm),
            Style::default().fg(theme().accent_action)
        );
        assert_eq!(
            style_from_name(Some("muted_text"), &sm),
            Style::default().fg(theme().muted_text)
        );
        assert_eq!(
            style_from_name(Some("streaming"), &sm),
            Style::default().fg(theme().streaming)
        );
        // And unknown names produce default style.
        assert_eq!(style_from_name(Some("unknown"), &sm), Style::default());
        assert_eq!(style_from_name(None, &sm), Style::default());
    }

    #[test]
    fn badge_ctx_carries_current_mode_string() {
        // Given an Input-mode state.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Input);
        let ctx = RenderCtx::new(&state);
        // When building the badge ctx.
        let v = build_badge_ctx(&ctx);
        // Then mode is the lowercase scope-mode string.
        assert_eq!(v["mode"], serde_json::json!("input"));
    }
}
