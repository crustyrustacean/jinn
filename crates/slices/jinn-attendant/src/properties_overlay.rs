//! The attendant properties popup — overlay geometry, view, and rows.
//!
//! The popup is a two-phase form. Its own scope is navigation-only: `j`/`k`
//! move the form cursor between the three fields, `h`/`l` pick a choice
//! within the focused field, and `i` (on the seed-template field) opens the
//! template editor on its own capturing scope. Every edit stays pending in
//! the popup's cell until `<enter>` commits all three fields to the session
//! together; `<esc>`/`<c-c>` restore the open-time snapshot and close.
//!
//! Both popup phases render the full form — only the top scope's overlay is
//! drawn, so the editor view re-assembles the same rows and adds the text
//! cursor. The shared row assembly lives in [`properties_view`].

use jinn_attendant_msg::{
    AttendantPropertiesState, PickDirection, PropertyField, attendant_seed_template_scope,
};
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{
    ActionCtx, ActionFn, BindSite, EditIntent, InputHook, RouteId, RouteOutcome, RouteRow,
    ScopeSignal,
};
use jinn_slices::{KeyRoutes, RenderFacts, RouteResult as IntentResult};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_segmentation::UnicodeSegmentation;

/// The typed cell the popup reads and writes.
type AttendantPropertiesCell = TypedCell<AttendantPropertiesState>;

/// Horizontal padding fraction for the popup (20% each side).
const POPUP_H_PAD_FRAC: f32 = 0.20;
/// Minimum popup width in cells.
const POPUP_MIN_WIDTH: u16 = 44;
/// Popup content height: three field rows, one hint line, one footer line.
const POPUP_CONTENT_ROWS: u16 = 5;

/// Computes the centered properties popup rectangle: title, three field
/// rows, one hint line, and a keybind footer.
fn properties_popup_rect(area: Rect) -> Rect {
    let popup_width = ((f32::from(area.width) * (1.0 - 2.0 * POPUP_H_PAD_FRAC)).ceil() as u16)
        .max(POPUP_MIN_WIDTH)
        .min(area.width);
    let popup_height = (POPUP_CONTENT_ROWS + 2u16).min(area.height);

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

/// The overlay view for the properties scope: the full form, no cursor.
///
/// The overlay only renders when the slice that owns the cell activated
/// (the `read_popup` bootstrap assertion covers the missing-cell case).
pub fn render_attendant_properties(frame: &mut Frame<'_>, area: Rect, ctx: &RenderFacts) {
    let popup = read_popup(ctx);
    let theme = &ctx.theme;

    frame.render_widget(Clear, area);
    render_popup_frame(frame, area, theme);
    let inner = inner_rect(area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    // The template row shows "> " plus the draft; the draft gets what's
    // left, minus one cell so the cursor can rest past the last grapheme.
    let template_width = inner.width.saturating_sub(2 + 1);
    frame.render_widget(
        Paragraph::new(properties_view(&popup, theme, template_width)),
        inner,
    );
    // Navigation-only scope: the properties view never sets the terminal
    // cursor. The template draft edits in the editor popup, which owns it.
}

/// The overlay view for the seed-template editor scope: the same form plus
/// the text cursor on the template row.
///
/// Only the top scope's overlay draws, so the editor view re-assembles the
/// properties rows. A missing overlay registration would blank the popup
/// while editing — the boot probe asserts the registration exists.
///
/// The editor only opens from the properties popup, which only renders
/// when the slice that owns the cell activated (the `read_popup` bootstrap
/// assertion covers the missing-cell case).
pub fn render_attendant_seed_template(frame: &mut Frame<'_>, area: Rect, ctx: &RenderFacts) {
    let popup = read_popup(ctx);
    let theme = &ctx.theme;

    frame.render_widget(Clear, area);
    render_popup_frame(frame, area, theme);
    let inner = inner_rect(area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    // The template row shows "> " plus the draft; the draft gets what's
    // left, minus one cell so the cursor can rest past the last grapheme.
    let template_width = inner.width.saturating_sub(2 + 1);
    frame.render_widget(
        Paragraph::new(properties_view(&popup, theme, template_width)),
        inner,
    );
    render_template_cursor(frame, inner, &popup);
}

/// Reads the popup's cell from the render facts.
///
/// # Panics
///
/// Panics if the slot is not registered — both overlays render only when
/// the attendant slice activated.
#[expect(
    clippy::expect_used,
    reason = "both overlays render only when the attendant slice registered the cell"
)]
fn read_popup(ctx: &RenderFacts) -> AttendantPropertiesState {
    ctx.slices
        .reader::<AttendantPropertiesState>(&jinn_attendant_msg::attendant_properties_slot())
        .expect("properties overlay renders only when the attendant cell is registered")
        .read()
        .clone()
}

/// Draws the popup's border and title. Shared by both phases.
fn render_popup_frame(frame: &mut Frame<'_>, area: Rect, theme: &jinn_theme::Theme) {
    let block = Block::default()
        .title(Span::styled(
            " Attendant Properties ",
            Style::default().fg(theme.popup_title),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_unfocused));
    frame.render_widget(block, area);
}

/// The popup body rect inside its border.
fn inner_rect(area: Rect) -> Rect {
    Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

/// Places the terminal cursor inside the template text, grapheme-safe.
///
/// The draft's row depends on which fields rendered a hint line above it —
/// the same layout [`properties_view`] assembles: each field before the
/// template contributes its own row, plus one more when it is focused
/// (its hint line).
fn render_template_cursor(frame: &mut Frame<'_>, inner: Rect, popup: &AttendantPropertiesState) {
    let prefix_len = 2u16;
    let grapheme_count = popup
        .seed_template
        .input
        .get(..popup.seed_template.cursor_pos)
        .map_or(0, |s| s.graphemes(true).count()) as u16;
    let cursor_x = (prefix_len + grapheme_count).min(inner.width.saturating_sub(1));
    let rows_above = {
        let hint_lines = u16::from(popup.focus != PropertyField::SeedTemplate);
        // Trigger row + activation row, plus their hint lines.
        2 + u16::from(popup.focus == PropertyField::Trigger) + hint_lines
    };
    let template_y = inner.y.saturating_add(rows_above);
    if template_y < inner.y + inner.height {
        frame.set_cursor_position((inner.x.saturating_add(cursor_x), template_y));
    }
}

/// Assembles the popup body from the popup state and theme.
///
/// One line per field: marker + label (yellow iff focused), then the
/// field's value spans. A hint line renders only under the focused field,
/// and the footer names the keys that work on the focused field. The
/// template draft truncates to `template_width` display columns,
/// grapheme-safe.
fn properties_view<'a>(
    popup: &'a AttendantPropertiesState,
    theme: &'a jinn_theme::Theme,
    template_width: u16,
) -> Vec<Line<'a>> {
    let mut lines = vec![];
    for field in [
        PropertyField::Trigger,
        PropertyField::Activation,
        PropertyField::SeedTemplate,
    ] {
        lines.push(field_line(popup, field, theme, template_width));
        if popup.focus == field {
            lines.push(hint_line(field, theme));
        }
    }
    lines.push(footer_line(popup.focus, theme));
    lines
}

/// One field row: focused marker + yellow label, then the value spans.
fn field_line<'a>(
    popup: &AttendantPropertiesState,
    field: PropertyField,
    theme: &'a jinn_theme::Theme,
    template_width: u16,
) -> Line<'a> {
    let focused = popup.focus == field;
    let mut spans = vec![
        field_marker(focused, theme),
        field_name(field, focused, theme),
    ];
    match field {
        PropertyField::Trigger => spans.extend(choice_spans(
            jinn_attendant_msg::TRIGGER_CHOICES,
            &popup.pending_trigger,
            theme,
        )),
        PropertyField::Activation => spans.extend(choice_spans(
            jinn_attendant_msg::ACTIVATION_CHOICES,
            &popup.pending_activation,
            theme,
        )),
        PropertyField::SeedTemplate => {
            spans.push(template_value(popup, theme, template_width));
        }
    }
    Line::from(spans)
}

/// The focused-field marker: `▸` when focused, a space otherwise.
fn field_marker(focused: bool, theme: &jinn_theme::Theme) -> Span<'static> {
    let marker = if focused { "▸ " } else { "  " };
    Span::styled(
        marker.to_owned(),
        Style::default().fg(if focused {
            theme.focus_accent
        } else {
            theme.primary_text
        }),
    )
}

/// The field's label, yellow only while its row is focused.
fn field_name(field: PropertyField, focused: bool, theme: &jinn_theme::Theme) -> Span<'static> {
    Span::styled(
        format!("{}:  ", field.label()),
        Style::default().fg(if focused {
            theme.focus_accent
        } else {
            theme.primary_text
        }),
    )
}

/// The choice spans for a choice row: the selected choice in the active
/// green, the rest in plain text, separated by muted slashes.
fn choice_spans<'a, T>(
    choices: &[(T, &'static str)],
    selected: &T,
    theme: &'a jinn_theme::Theme,
) -> Vec<Span<'a>>
where
    T: PartialEq,
{
    let mut spans = vec![];
    for (index, (value, label)) in choices.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" / ", Style::default().fg(theme.muted_text)));
        }
        let style = if *value == *selected {
            Style::default().fg(theme.attendant_option_active)
        } else {
            Style::default().fg(theme.primary_text)
        };
        spans.push(Span::styled((*label).to_owned(), style));
    }
    spans
}

/// The template text span: the live draft, in plain text color, truncated
/// to `max_width` display columns by grapheme (never by byte or char).
fn template_value<'a>(
    popup: &AttendantPropertiesState,
    theme: &'a jinn_theme::Theme,
    max_width: u16,
) -> Span<'a> {
    let max_graphemes = usize::from(max_width);
    let truncated: String = popup
        .seed_template
        .input
        .graphemes(true)
        .take(max_graphemes)
        .collect();
    Span::styled(truncated, Style::default().fg(theme.primary_text))
}

/// The hint line under the focused field.
fn hint_line(field: PropertyField, theme: &jinn_theme::Theme) -> Line<'static> {
    let hint = match field {
        PropertyField::Trigger => "does this attendant re-run when its parent's turn completes?",
        PropertyField::Activation => {
            "seed pins without dispatching · reset keeps only pins · continue appends"
        }
        PropertyField::SeedTemplate => "the text injected ahead of each run's prior report",
    };
    Line::from(Span::styled(
        format!("    {hint}"),
        Style::default().fg(theme.muted_text),
    ))
}

/// The footer: the keys that work on the focused field.
fn footer_line(focus: PropertyField, theme: &jinn_theme::Theme) -> Line<'static> {
    let keys = match focus {
        PropertyField::Trigger | PropertyField::Activation => {
            "h/l pick · j/k field · <enter> apply · <esc> cancel"
        }
        PropertyField::SeedTemplate => "i edit · j/k field · <enter> apply · <esc> cancel",
    };
    Line::from(Span::styled(keys, Style::default().fg(theme.muted_text)))
}

/// Builds the popup's input hook: typed keys edit the seed template draft.
///
/// Registered against the seed-template editor's scope (the properties
/// scope captures nothing) — or typed characters would reach nothing.
#[must_use]
pub fn attendant_properties_input_hook(cell: &AttendantPropertiesCell) -> InputHook {
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

/// Builds an own-scope row for one of the popup's scopes.
fn row(
    route_id: &'static str,
    scope: jinn_slices::SliceScopeId,
    key: &'static str,
    category: &'static str,
    display: &'static str,
    run: ActionFn,
) -> RouteRow {
    RouteRow {
        route_id: RouteId::new(route_id),
        scope,
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

/// Attaches every row the properties form owns.
///
/// The `P` opener lives with the sidebar's sessions rows, because `P` means
/// "edit the highlighted session's attendant" there. The editor's keep/
/// restore/clear rows land on the editor scope in
/// [`attach_seed_template_rows`]; this function owns the form itself.
pub fn attach_properties_rows(routes: &KeyRoutes, cell: &AttendantPropertiesCell) {
    let properties_scope = jinn_attendant_msg::attendant_properties_scope();

    routes.attach(row(
        "attendant-properties-field-next",
        properties_scope.clone(),
        "j",
        "navigation",
        "next field",
        action(cell, |_, cell| {
            cell.update(AttendantPropertiesState::focus_next);
            IntentResult::empty()
        }),
    ));
    routes.attach(row(
        "attendant-properties-field-previous",
        properties_scope.clone(),
        "k",
        "navigation",
        "previous field",
        action(cell, |_, cell| {
            cell.update(AttendantPropertiesState::focus_previous);
            IntentResult::empty()
        }),
    ));
    routes.attach(row(
        "attendant-properties-pick-left",
        properties_scope.clone(),
        "h",
        "navigation",
        "pick the previous choice",
        action(cell, |_, cell| {
            cell.update(|popup| popup.pick(PickDirection::Left));
            IntentResult::empty()
        }),
    ));
    routes.attach(row(
        "attendant-properties-pick-right",
        properties_scope.clone(),
        "l",
        "navigation",
        "pick the next choice",
        action(cell, |_, cell| {
            cell.update(|popup| popup.pick(PickDirection::Right));
            IntentResult::empty()
        }),
    ));
    routes.attach(row(
        "attendant-properties-edit-template",
        properties_scope.clone(),
        "i",
        "general",
        "edit the seed template",
        action(cell, |_ctx, cell| {
            // `i` means "edit the template" only from the template field.
            if cell.read().focus != PropertyField::SeedTemplate {
                return IntentResult::empty();
            }
            cell.update(AttendantPropertiesState::begin_template_edit);
            IntentResult::empty()
                .with_scope_signal(ScopeSignal::Push(attendant_seed_template_scope()))
        }),
    ));
    routes.attach(row(
        "attendant-properties-apply",
        properties_scope.clone(),
        "<enter>",
        "general",
        "apply all fields and close",
        action(cell, apply_all_fields),
    ));
    for (route_id, key) in [
        ("attendant-properties-leave", "<esc>"),
        ("attendant-properties-cancel", "<c-c>"),
    ] {
        routes.attach(row(
            route_id,
            properties_scope.clone(),
            key,
            "general",
            "restore originals and close",
            action(cell, |_, cell| {
                cell.update(AttendantPropertiesState::restore_original);
                IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(
                    jinn_attendant_msg::attendant_properties_scope(),
                ))
            }),
        ));
    }
}

/// The apply row's action: commits the popup's pending values to the
/// session it names, together, and persists once.
fn apply_all_fields(ctx: &mut ActionCtx<'_>, cell: &AttendantPropertiesCell) -> IntentResult {
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    let popup = cell.read().clone();
    let Some(attendant_id) = popup.session_id.clone() else {
        return IntentResult::empty();
    };
    let template = popup.seed_template.input.clone();
    let (activation, trigger) = (popup.pending_activation, popup.pending_trigger);
    let Some(session) = state.session.get_mut(&attendant_id) else {
        return IntentResult::empty();
    };
    session.set_seed_template(template);
    session.set_attendant_activation(activation);
    session.set_attendant_trigger(trigger);
    // A fresh attendant was never interacted; without this the persist
    // below is silently dropped.
    session.mark_interacted();
    session.touch();
    IntentResult::empty()
        .with_message(jinn_session_store_msg::PersistSession {
            session_id: attendant_id,
        })
        .with_scope_signal(ScopeSignal::PopIf(
            jinn_attendant_msg::attendant_properties_scope(),
        ))
}

/// Attaches the template editor's keep/restore/clear rows on its own scope.
pub fn attach_seed_template_rows(routes: &KeyRoutes, cell: &AttendantPropertiesCell) {
    let editor_scope = attendant_seed_template_scope();

    routes.attach(row(
        "attendant-template-keep",
        editor_scope.clone(),
        "<enter>",
        "general",
        "keep the edited template",
        action(cell, |_, cell| {
            cell.update(AttendantPropertiesState::keep_template_edit);
            IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(
                jinn_attendant_msg::attendant_seed_template_scope(),
            ))
        }),
    ));
    routes.attach(row(
        "attendant-template-restore",
        editor_scope.clone(),
        "<esc>",
        "general",
        "restore the pre-editor template",
        action(cell, |_, cell| {
            cell.update(AttendantPropertiesState::cancel_template_edit);
            IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(
                jinn_attendant_msg::attendant_seed_template_scope(),
            ))
        }),
    ));
    routes.attach(row(
        "attendant-template-clear-or-leave",
        editor_scope,
        "<c-c>",
        "general",
        "clear the template, or leave when already empty",
        action(cell, |_, cell| {
            let was_empty = cell.update(|popup| {
                let was_empty = popup.seed_template.input.is_empty();
                if !was_empty {
                    // Clearing *is* the edit: the user wiped the text, so the
                    // draft stands rather than being restored away. This
                    // deliberately diverges from the rename popup, whose
                    // `<c-c>` preserves the input state for a later restore.
                    popup.seed_template = jinn_slices::LineInput::default();
                    popup.editor_original = None;
                }
                was_empty
            });
            if was_empty {
                IntentResult::empty().with_scope_signal(ScopeSignal::PopIf(
                    jinn_attendant_msg::attendant_seed_template_scope(),
                ))
            } else {
                IntentResult::empty()
            }
        }),
    ));
}

/// Registers the template editor's input hook on the editor scope.
///
/// The properties scope captures no input, so typed characters reach the
/// template draft only through this registration.
pub fn register_seed_template_input_hook(routes: &KeyRoutes, cell: &AttendantPropertiesCell) {
    let hook = attendant_properties_input_hook(cell);
    routes.register_input_hook(&jinn_attendant_msg::attendant_seed_template_scope(), hook);
}
