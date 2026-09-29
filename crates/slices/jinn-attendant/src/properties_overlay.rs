//! The attendant properties popup — overlay geometry, view, and input hook.
//!
//! Follows the rename-session popup end to end: a geometry fn computes the
//! centered rect, the view draws the three controls (trigger, activation,
//! seed template) reading edit state from the popup's cell, and the input
//! hook routes typed keys into the seed-template `LineInput` while the
//! popup's scope is active.

use jinn_attendant_msg::{AttendantActivation, AttendantPropertiesState, AttendantTrigger};
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{
    ActionCtx, ActionFn, BindSite, EditIntent, InputHook, RouteOutcome, RouteRow, ScopeSignal,
};
use jinn_slices::{KeyRoutes, RenderFacts, RouteId, RouteResult as IntentResult};

/// The typed cell the popup reads and writes.
type AttendantPropertiesCell = TypedCell<AttendantPropertiesState>;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_segmentation::UnicodeSegmentation;

/// Horizontal padding fraction for the popup (20% each side).
const POPUP_H_PAD_FRAC: f32 = 0.20;
/// Minimum popup width in cells.
const POPUP_MIN_WIDTH: u16 = 44;

/// Computes the centered properties popup rectangle: title, three control
/// rows, the seed-template input, and a keybind footer.
fn properties_popup_rect(area: Rect) -> Rect {
    let popup_width = ((f32::from(area.width) * (1.0 - 2.0 * POPUP_H_PAD_FRAC)).ceil() as u16)
        .max(POPUP_MIN_WIDTH)
        .min(area.width);
    let popup_height = 9u16.min(area.height);

    #[expect(clippy::integer_division, reason = "cell positions are integers")]
    let popup_x = area.width.saturating_sub(popup_width) / 2;
    #[expect(clippy::integer_division, reason = "cell positions are integers")]
    let popup_y = area.height.saturating_sub(popup_height) / 3;

    Rect::new(popup_x, popup_y, popup_width, popup_height)
}

/// The overlay-rect function registered on the slice host.
pub fn attendant_properties_overlay_rect(area: &Rect) -> Option<Rect> {
    Some(properties_popup_rect(*area))
}

/// The overlay view: draws the popup into `area` (the geometry rect).
///
/// # Panics
///
/// Panics if the attendant properties slot is not registered — the overlay
/// only renders when the slice that owns it activated.
#[expect(
    clippy::expect_used,
    reason = "the overlay only renders when the attendant slice registered its cell"
)]
pub fn render_attendant_properties(frame: &mut Frame<'_>, area: Rect, ctx: &RenderFacts) {
    let cell: TypedCell<AttendantPropertiesState> = ctx
        .slices
        .reader(&jinn_attendant_msg::attendant_properties_slot())
        .expect("properties overlay renders only when the attendant cell is registered");
    let popup = cell.read();
    let theme = &ctx.theme;

    let (trigger_label, trigger_hint) = trigger_control(popup.trigger_focus);
    let (activation_label, activation_hint) = activation_control(popup.activation_focus);

    let rows = vec![
        Line::from(Span::styled(
            trigger_label,
            Style::default().fg(if popup.trigger_focus {
                theme.focus_accent
            } else {
                theme.primary_text
            }),
        )),
        Line::from(Span::styled(
            format!("    {trigger_hint}"),
            Style::default().fg(theme.muted_text),
        )),
        Line::from(Span::styled(
            activation_label,
            Style::default().fg(if popup.activation_focus {
                theme.focus_accent
            } else {
                theme.primary_text
            }),
        )),
        Line::from(Span::styled(
            format!("    {activation_hint}"),
            Style::default().fg(theme.muted_text),
        )),
        Line::from(Span::styled(
            "seed template:",
            Style::default().fg(theme.primary_text),
        )),
        Line::from(vec![
            Span::styled("> ", Style::default().fg(theme.focus_accent)),
            Span::raw(&popup.seed_template.input),
        ]),
        Line::from(Span::styled(
            "<tab> next field · <enter> cycle focused toggle · <esc> done",
            Style::default().fg(theme.muted_text),
        )),
    ];

    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(Span::styled(
            " Attendant Properties ",
            Style::default().fg(theme.popup_title),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_unfocused));
    frame.render_widget(block, area);

    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    frame.render_widget(Paragraph::new(rows), inner);

    // The cursor sits on the seed-template line only while the template is
    // the focused field (neither toggle is).
    if !popup.trigger_focus && !popup.activation_focus {
        let prefix_len = 2u16;
        let grapheme_count = popup
            .seed_template
            .input
            .get(..popup.seed_template.cursor_pos)
            .map_or(0, |s| s.graphemes(true).count()) as u16;
        let cursor_x = (prefix_len + grapheme_count).min(inner.width.saturating_sub(1));
        // The template line is the 6th row (index 5) of the popup body.
        let template_y = inner.y.saturating_add(5);
        if template_y < inner.y + inner.height {
            frame.set_cursor_position((inner.x.saturating_add(cursor_x), template_y));
        }
    }
}

fn trigger_control(focused: bool) -> (String, &'static str) {
    let marker = if focused { "▸" } else { " " };
    (
        format!("{marker} trigger:  parent-completed / manual"),
        "does this attendant re-run when its parent's turn completes?",
    )
}

fn activation_control(focused: bool) -> (String, &'static str) {
    let marker = if focused { "▸" } else { " " };
    (
        format!("{marker} activation: seed / reset / continue"),
        "seed pins without dispatching · reset keeps only pins · continue appends",
    )
}

/// Builds the popup's input hook: typed keys edit the seed template.
///
/// Must be registered against the popup's scope, or the popup swallows
/// input without effect.
#[must_use]
pub fn attendant_properties_input_hook(cell: &TypedCell<AttendantPropertiesState>) -> InputHook {
    let cell = cell.clone();
    std::sync::Arc::new(move |intent: &EditIntent| {
        let cell = cell.clone();
        match intent {
            EditIntent::InsertChar(ch) => {
                cell.update(|s| s.seed_template.insert_char(*ch));
                Some(IntentResult::empty())
            }
            EditIntent::DeleteBackward => {
                cell.update(|s| s.seed_template.delete());
                Some(IntentResult::empty())
            }
            EditIntent::DeleteForward => {
                cell.update(|s| s.seed_template.delete_forward());
                Some(IntentResult::empty())
            }
            EditIntent::Paste(text) => {
                let text = text.clone();
                cell.update(move |s| {
                    for ch in text.chars() {
                        s.seed_template.insert_char(ch);
                    }
                });
                Some(IntentResult::empty())
            }
            EditIntent::CursorLeft => {
                cell.update(|s| s.seed_template.cursor_left());
                Some(IntentResult::empty())
            }
            EditIntent::CursorRight => {
                cell.update(|s| s.seed_template.cursor_right());
                Some(IntentResult::empty())
            }
            EditIntent::CursorHome => {
                cell.update(|s| s.seed_template.cursor_home());
                Some(IntentResult::empty())
            }
            EditIntent::CursorEnd => {
                cell.update(|s| s.seed_template.cursor_end());
                Some(IntentResult::empty())
            }
        }
    })
}

/// The three toggle/cycle behaviors, pure over the popup state, so the route
/// rows stay thin and the tests stay table-level.
///
/// Focus the next field: trigger → activation → seed template → trigger.
pub fn cycle_focus(popup: &mut AttendantPropertiesState) {
    if popup.trigger_focus {
        popup.trigger_focus = false;
        popup.activation_focus = true;
    } else if popup.activation_focus {
        popup.activation_focus = false;
        // Seed template is the focus by default (both flags false).
    } else {
        popup.trigger_focus = true;
    }
}

/// Cycle the focused toggle's value, returning the (activation, trigger)
/// pair to apply. The seed template is edited in place, not cycled.
pub fn cycle_focused_value(
    popup: &mut AttendantPropertiesState,
    current: (AttendantActivation, AttendantTrigger),
) -> (AttendantActivation, AttendantTrigger) {
    let (activation, trigger) = current;
    if popup.trigger_focus {
        let next = match trigger {
            AttendantTrigger::Manual => AttendantTrigger::ParentCompleted,
            AttendantTrigger::ParentCompleted => AttendantTrigger::Manual,
        };
        return (activation, next);
    }
    if popup.activation_focus {
        let next = match activation {
            AttendantActivation::Seed => AttendantActivation::Reset,
            AttendantActivation::Reset => AttendantActivation::Continue,
            AttendantActivation::Continue => AttendantActivation::Seed,
        };
        return (next, trigger);
    }
    (activation, trigger)
}

/// The kernel's application state behind an [`ActionCtx`].
fn app<'a>(ctx: &'a mut ActionCtx<'_>) -> Option<&'a mut jinn_kernel::AppState> {
    ctx.state
        .as_any_mut()?
        .downcast_mut::<jinn_kernel::AppState>()
}

/// Wraps a popup action in an [`ActionFn`], handing it the cell.
fn action<F>(cell: &AttendantPropertiesCell, f: F) -> ActionFn
where
    F: Fn(&mut ActionCtx<'_>, &AttendantPropertiesCell) -> IntentResult + Send + Sync + 'static,
{
    let cell = cell.clone();
    ActionFn::new(move |mut ctx| f(&mut ctx, &cell))
}

/// Builds an own-scope row bound to a popup action.
fn row(
    route_id: &'static str,
    key: &'static str,
    category: &'static str,
    display: &'static str,
    run: ActionFn,
) -> RouteRow {
    RouteRow {
        route_id: RouteId::new(route_id),
        scope: jinn_attendant_msg::attendant_properties_scope(),
        key,
        category,
        site: BindSite::OwnScope,
        feature: "attendant",
        outcome: RouteOutcome::Action {
            action: route_id,
            display,
            run,
        },
    }
}

/// Attaches every row the properties popup owns (editing keys, confirm,
/// leave). The `P` opener lives with the sidebar's sessions rows, because
/// `P` means "edit the highlighted session's attendant" there.
pub fn attach_properties_rows(routes: &KeyRoutes, cell: &AttendantPropertiesCell) {
    // Editing keys on the popup's own scope. The input hook covers typed
    // characters; these cover the structural keys.
    routes.attach(row(
        "attendant-properties-field-next",
        "<tab>",
        "input",
        "next field",
        action(cell, |_, cell| {
            cell.update(properties_overlay_inner::cycle_focus);
            IntentResult::empty()
        }),
    ));
    routes.attach(row(
        "attendant-properties-cycle",
        "<enter>",
        "general",
        "cycle focused toggle / apply",
        action(cell, |ctx, cell| {
            let Some(state) = app(ctx) else {
                return IntentResult::empty();
            };
            // Apply: the popup holds the pending values; commit them to the
            // session it names and persist.
            let popup = cell.read().clone();
            let Some(attendant_id) = popup.session_id.clone() else {
                return IntentResult::empty();
            };
            let (template, activation, trigger) = properties_overlay_inner::apply(&popup);
            let Some(session) = state.session.get_mut(&attendant_id) else {
                return IntentResult::empty();
            };
            session.set_seed_template(template);
            session.set_attendant_activation(activation);
            session.set_attendant_trigger(trigger);
            IntentResult::empty().with_message(jinn_session_store_msg::PersistSession {
                session_id: attendant_id,
            })
        }),
    ));
    routes.attach(row(
        "attendant-properties-leave",
        "<esc>",
        "general",
        "close without applying",
        ActionFn::new(|_ctx| {
            IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(
                jinn_attendant_msg::attendant_properties_scope(),
            ))
        }),
    ));
}

mod properties_overlay_inner {
    //! The pure state helpers the route rows call into.
    pub use super::cycle_focus;

    /// Commits the popup's edits: the template text and the current toggle
    /// pair. `<enter>` on a focused toggle cycles *before* this runs, so
    /// apply is idempotent.
    pub fn apply(
        popup: &super::AttendantPropertiesState,
    ) -> (String, super::AttendantActivation, super::AttendantTrigger) {
        (
            popup.seed_template.input.clone(),
            popup.current_activation,
            popup.current_trigger,
        )
    }
}
