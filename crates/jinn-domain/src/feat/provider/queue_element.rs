//! Queue display element.
//!
//! Renders stacked dimmed "QUEUED: ⟨first line⟩" entries above the input box
//! when messages are waiting in the queue.

use crate::common::render_ctx::RenderCtx;
use crate::common::ui_element::UiElement;

use crate::protocol::ChatEntryKind;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_segmentation::UnicodeSegmentation as _;

/// Displays queued messages as dimmed entries.
#[derive(Debug)]
pub struct QueueDisplayElement;

impl UiElement for QueueDisplayElement {
    fn name(&self) -> String {
        "queue-display".to_owned()
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
        let state = ctx.state;
        let queue = state.active_session().queue();
        if queue.is_empty() {
            return;
        }

        let lines: Vec<Line> = queue
            .iter()
            .filter_map(|item| match item {
                crate::feat::session::queue_item::QueueItem::UserMessage(entry) => {
                    let display_text = match &entry.kind {
                        ChatEntryKind::User { display, .. } => display.as_str(),
                        _ => "",
                    };
                    let first_line = display_text.lines().next().unwrap_or("");
                    let display = if first_line.len() > 60 {
                        let truncated: String = first_line.graphemes(true).take(59).collect();
                        format!("QUEUED: {truncated}…")
                    } else {
                        format!("QUEUED: {first_line}")
                    };
                    Some(Line::from(Span::styled(
                        display,
                        Style::default().fg(state.frontend.theme.muted_text),
                    )))
                }
                crate::feat::session::queue_item::QueueItem::ToolContinuation => None,
            })
            .collect();

        let paragraph = Paragraph::new(lines);
        frame.render_widget(paragraph, area);
    }
}
