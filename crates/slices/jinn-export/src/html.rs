//! The HTML export format.
//!
//! Produces one self-contained file: inline CSS, no script, no external
//! font, and every image inlined as a `data:` URI. The file opens correctly
//! from disk with no network and no sibling assets, which is the whole point
//! of an export — it has to survive being emailed.
//!
//! Dark is the default palette; a `prefers-color-scheme: light` block
//! overrides it, so the document reads correctly in either. There is no
//! toggle control and no JavaScript, deliberately.
//!
//! Prose (user, assistant, actor, system, error, compaction summaries) is
//! rendered from markdown via `pulldown-cmark`. Tool arguments, tool results,
//! and reasoning are *not* markdown — they are JSON, tool output, and model
//! scratchwork — so they go into `<pre><code>`, HTML-escaped. Every string
//! reaching the output is either escaped or emitted by the markdown
//! renderer, never interpolated into markup by hand.
//!
//! Terminal affordances are not reproduced. There is no gutter, no pin
//! glyph, no "press h to show" hint, and no `---(N lines hidden above)---`
//! marker: the tail-fold a terminal needs has no meaning in a scrolling
//! document, where the body is already a click away.

use pulldown_cmark::{Event, Options, Parser};

use crate::document::ExportDocument;
use crate::document::ExportEntry;
use crate::document::ExportToolStatus;
use crate::format::ExportFormat;

/// Escapes text for interpolation into HTML character data or an attribute
/// value.
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// The inlined stylesheet: dark by default, light by media query.
const STYLESHEET: &str = "\
:root {
  --bg: #16181d;
  --fg: #d7dae0;
  --muted: #8b919e;
  --rule: #2b2f38;
  --user: #7fb3ff;
  --assistant: #6fd6a8;
  --actor: #e2c76b;
  --system: #8b919e;
  --error: #f08a8a;
  --thinking: #c9a6f0;
  --tool: #7fd4d6;
  --surface: #1d2027;
}
@media (prefers-color-scheme: light) {
  :root {
    --bg: #ffffff;
    --fg: #1c1f24;
    --muted: #5f6672;
    --rule: #dfe3e8;
    --user: #1a5fb4;
    --assistant: #1a7f4b;
    --actor: #8a6d00;
    --system: #5f6672;
    --error: #b3261e;
    --thinking: #7b3fbf;
    --tool: #0f6f73;
    --surface: #f4f6f8;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0 auto;
  padding: 2rem 1.25rem 6rem;
  max-width: 52rem;
  background: var(--bg);
  color: var(--fg);
  font: 16px/1.65 -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
}
h1 { font-size: 1.5rem; margin: 0 0 .25rem; }
h2 { font-size: 1.15rem; margin: 2rem 0 .5rem; }
code, pre { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
pre {
  background: var(--surface);
  border: 1px solid var(--rule);
  border-radius: 6px;
  padding: .7rem .85rem;
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-word;
  font-size: .85rem;
}
a { color: var(--user); }
hr { border: 0; border-top: 1px solid var(--rule); margin: 2rem 0; }
.meta { color: var(--muted); font-size: .85rem; }
.meta dt { display: inline; }
.meta dd { display: inline; margin: 0 1rem 0 .25rem; }
.entry { margin: 1.1rem 0; }
.entry-user { border-left: 3px solid var(--user); padding-left: .8rem; }
.entry-assistant { border-left: 3px solid var(--assistant); padding-left: .8rem; }
.entry-actor { border-left: 3px solid var(--actor); padding-left: .8rem; }
.entry-system, .entry-error { color: var(--muted); border-left: 3px solid var(--rule); padding-left: .8rem; }
.entry-error { color: var(--error); border-left-color: var(--error); }
.entry-thinking, .entry-tool_call, .entry-tool_result { border-left: 3px solid var(--tool); padding-left: .8rem; }
.entry-thinking { border-left-color: var(--thinking); }
.entry-compaction, .entry-annotation { border-left: 3px solid var(--actor); padding-left: .8rem; }
.entry > .label { font-size: .78rem; text-transform: uppercase; letter-spacing: .06em; color: var(--muted); }
details > summary { cursor: pointer; font-size: .9rem; color: var(--tool); }
details[open] > summary { margin-bottom: .5rem; }
details > summary .status-failure { color: var(--error); }
details > summary .status-pending { color: var(--actor); }
img { max-width: 100%; border: 1px solid var(--rule); border-radius: 6px; margin: .4rem 0; }
blockquote { margin: .5rem 0; padding-left: .9rem; border-left: 3px solid var(--rule); color: var(--muted); }
table { border-collapse: collapse; }
td, th { border: 1px solid var(--rule); padding: .3rem .5rem; }
ul, ol { padding-left: 1.4rem; }
";

/// Renders markdown source to HTML.
///
/// Raw HTML in the source is escaped rather than passed through, so a
/// `<script>` written by a model (or pasted into a message) stays inert text
/// in the export instead of becoming live markup. Everything else is
/// delegated to pulldown-cmark's own renderer, which escapes text nodes.
fn render_markdown(source: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    // Raw HTML in the source is demoted to a text node rather than passed
    // through, so a `<script>` written by a model (or pasted into a message)
    // stays inert text in the export instead of becoming live markup. The
    // whole event stream is rewritten first and rendered in one pass, so the
    // library's writer keeps a single consistent state.
    let events = Parser::new_ext(source, options).map(|event| match event {
        Event::Html(text) => Event::Text(text),
        other => other,
    });
    let mut out = String::with_capacity(source.len() + source.len() / 4);
    pulldown_cmark::html::push_html(&mut out, events);
    out
}

/// The visible label above a prose entry.
fn prose_label(class: &str) -> &'static str {
    match class {
        "user" => "You",
        "assistant" => "Assistant",
        "actor" => "Actor",
        "system" => "System",
        "error" => "Error",
        "compaction" => "Compaction summary",
        "annotation" => "Sources",
        _ => "Entry",
    }
}

/// A stable anchor id for the entry at `index`.
fn anchor_id(index: usize) -> String {
    format!("entry-{index}")
}

/// Renders one entry as a disclosure block.
///
/// The summary is the precomputed one-line label; the body is the full text
/// in a `<pre><code>`, HTML-escaped. No tail-fold: the whole payload is two
/// clicks away and truncation is a terminal constraint, not a document one.
fn render_disclosure(entry: &ExportEntry) -> String {
    let summary = entry
        .summary
        .as_deref()
        .map_or_else(|| entry.class.to_owned(), std::borrow::ToOwned::to_owned);
    format!(
        "<details><summary><span class=\"summary-label\">{summary}</span>\
         <span class=\"kind\">{class}</span>{status}</summary>\
         <pre><code>{body}</code></pre></details>",
        summary = escape_html(&summary),
        class = escape_html(entry.class),
        status = render_status_badge(entry.tool_status),
        body = escape_html(&entry.body),
    )
}

/// The coloured outcome word shown in a tool-result summary line.
fn render_status_badge(status: Option<ExportToolStatus>) -> String {
    status.map_or_else(String::new, |status| {
        format!(
            r#" <span class="status-{}">{}</span>"#,
            status.as_str(),
            status.as_str()
        )
    })
}

/// Renders one entry as inline prose plus any inlined images.
fn render_prose(entry: &ExportEntry, anchor: &str) -> String {
    let mut out = format!(
        "<div class=\"entry entry-{}\" id=\"{anchor}\">",
        escape_html(entry.class)
    );
    out.push_str("<div class=\"label\">");
    out.push_str(&escape_html(prose_label(entry.class)));
    out.push_str("</div>");
    out.push_str(&render_markdown(&entry.body));
    for image in &entry.images {
        out.push_str("\n<img src=\"");
        out.push_str(&escape_html(&image.data_url()));
        out.push_str("\" alt=\"attached image\" />");
    }
    out.push_str("</div>");
    out
}

/// The session header: title plus the metadata a reader needs to place it.
fn render_header(doc: &ExportDocument) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "<h1>{}</h1>", escape_html(&doc.title));
    out.push_str("<dl class=\"meta\">");
    let _ = write!(out, "<dt>model</dt><dd>{}</dd>", escape_html(&doc.model));
    let _ = write!(
        out,
        "<dt>working directory</dt><dd>{}</dd>",
        escape_html(&doc.cwd.display().to_string())
    );
    let _ = write!(
        out,
        "<dt>created</dt><dd>{}</dd>",
        escape_html(&doc.created_at.to_string())
    );
    let _ = write!(
        out,
        "<dt>updated</dt><dd>{}</dd>",
        escape_html(&doc.updated_at.to_string())
    );
    let _ = write!(out, "<dt>entries</dt><dd>{}</dd>", doc.entries.len());
    out.push_str("</dl>\n");

    out
}

/// Renders a whole document as a standalone HTML file.
#[must_use]
pub fn render_html(doc: &ExportDocument) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(16 * 1024);
    out.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
    out.push_str("<meta charset=\"utf-8\" />\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n");
    let _ = writeln!(out, "<title>{}</title>", escape_html(&doc.title));
    out.push_str("<style>\n");
    out.push_str(STYLESHEET);
    out.push_str("</style>\n</head>\n<body>\n");

    out.push_str("<header>\n");
    out.push_str(&render_header(doc));
    out.push_str("</header>\n");

    out.push_str("<main>\n");
    for (index, entry) in doc.entries.iter().enumerate() {
        let anchor = anchor_id(index);
        if entry.is_disclosure {
            let _ = writeln!(
                out,
                "<div class=\"entry entry-{class}\" id=\"{anchor}\">",
                class = escape_html(entry.class),
                anchor = anchor,
            );
            out.push_str(&render_disclosure(entry));
            out.push_str("\n</div>\n");
        } else {
            out.push_str(&render_prose(entry, &anchor));
            out.push('\n');
        }
    }
    out.push_str("</main>\n</body>\n</html>\n");
    out
}

/// The HTML [`ExportFormat`].
///
/// Named only here and in `format.rs`'s resolver arm; no other call site
/// knows this type exists.
#[derive(Debug, Clone, Copy)]
pub struct HtmlExport;

impl ExportFormat for HtmlExport {
    fn extension(&self) -> &'static str {
        "html"
    }

    fn render(&self, doc: &ExportDocument) -> String {
        render_html(doc)
    }
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
    use super::*;
    use crate::document::ExportImage;
    use jiff::Timestamp;

    fn doc_with(entries: Vec<ExportEntry>) -> ExportDocument {
        ExportDocument {
            title: "Test session".to_owned(),
            model: "test-model".to_owned(),
            cwd: std::path::PathBuf::from("/tmp/work"),
            created_at: Timestamp::UNIX_EPOCH,
            updated_at: Timestamp::UNIX_EPOCH,
            session_id: "s-1".to_owned(),
            entries,
        }
    }

    fn entry_of(class: &'static str, body: &str) -> ExportEntry {
        ExportEntry {
            class,
            body: body.to_owned(),
            summary: None,
            is_disclosure: false,
            images: Vec::new(),
            tool_status: None,
            line_count: None,
            token_count: None,
            timestamp: Timestamp::UNIX_EPOCH,
        }
    }

    /// Collects every `name="value"` attribute value found in `html`.
    ///
    /// Walks with `match_indices`/`get` rather than indexing, so it cannot
    /// panic on a multi-byte character boundary.
    fn attribute_values<'a>(html: &'a str, name: &'a str) -> Vec<(&'a str, &'a str)> {
        let needle = format!("{name}=\"");
        let open = needle.len();
        html.match_indices(&needle)
            .filter_map(|(at, _)| {
                let rest = html.get(at + open..)?;
                let end = rest.find('"')?;
                Some((name, rest.get(..end)?))
            })
            .collect()
    }

    #[rstest::rstest]
    fn output_references_no_external_resource() {
        // Given a document with an inlined image and a prose entry.
        let mut image_entry = entry_of("user", "look");
        image_entry.images = vec![ExportImage {
            media_type: "image/png".to_owned(),
            data: vec![1, 2, 3],
        }];
        let doc = doc_with(vec![image_entry, entry_of("assistant", "hi")]);

        // When rendering it to HTML.
        let html = render_html(&doc);

        // Then every src/href is a data: URI or an in-page fragment.
        let refs = attribute_values(&html, "src");
        let refs = refs.into_iter().chain(attribute_values(&html, "href"));
        for (attr, value) in refs {
            assert!(
                value.starts_with("data:") || value.starts_with('#'),
                "external reference found: {attr}=\"{value}\""
            );
        }
    }

    #[rstest::rstest]
    fn each_disclosure_kind_renders_as_a_details_element() {
        // Given one tool call, one tool result, and one reasoning entry.
        let mut tool_call = entry_of("tool_call", r#"{"cmd":"ls"}"#);
        tool_call.is_disclosure = true;
        tool_call.summary = Some("bash · ls".to_owned());
        let mut tool_result = entry_of("tool_result", "output");
        tool_result.is_disclosure = true;
        tool_result.summary = Some("read · success · 1 lines".to_owned());
        let mut thinking = entry_of("thinking", "considering");
        thinking.is_disclosure = true;
        thinking.summary = Some("reasoning · ~12 tokens".to_owned());
        let doc = doc_with(vec![tool_call, tool_result, thinking]);

        // When rendering it.
        let html = render_html(&doc);

        // Then all three collapse into details elements.
        assert_eq!(html.matches("<details>").count(), 3);
        assert!(html.contains("bash · ls"));
        assert!(html.contains("read · success · 1 lines"));
        assert!(html.contains("reasoning · ~12 tokens"));
    }

    #[rstest::rstest]
    fn script_in_assistant_text_is_escaped() {
        // Given an assistant message containing a script tag.
        let doc = doc_with(vec![entry_of("assistant", "<script>alert('xss')</script>")]);

        // When rendering it.
        let html = render_html(&doc);

        // Then it is inert escaped text, not live markup.
        assert!(!html.contains("<script>alert"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[rstest::rstest]
    fn stylesheet_carries_a_light_scheme_override() {
        // Given any document.
        let doc = doc_with(vec![entry_of("assistant", "hi")]);

        // When rendering it.
        let html = render_html(&doc);

        // Then the stylesheet adapts to a light colour scheme.
        assert!(html.contains("@media (prefers-color-scheme: light)"));
    }

    #[rstest::rstest]
    fn markdown_heading_in_assistant_text_becomes_a_heading() {
        // Given an assistant message with a markdown heading.
        let doc = doc_with(vec![entry_of("assistant", "## Findings")]);

        // When rendering it.
        let html = render_html(&doc);

        // Then it becomes a real heading element.
        assert!(html.contains("<h2>Findings</h2>"));
    }

    #[rstest::rstest]
    fn every_entry_is_still_rendered_when_there_is_no_contents_list() {
        // Given a three-entry document.
        let doc = doc_with(vec![
            entry_of("user", "first message"),
            entry_of("assistant", "second message"),
            entry_of("actor", "third message"),
        ]);

        // When rendering it.
        let html = render_html(&doc);

        // Then all three bodies are present, in order.
        let first = html.find("first message").expect("first entry");
        let second = html.find("second message").expect("second entry");
        let third = html.find("third message").expect("third entry");
        assert!(first < second && second < third);
    }

    #[rstest::rstest]
    fn the_header_has_no_table_of_contents() {
        // Given a document with several entries.
        let doc = doc_with(vec![
            entry_of("user", "one"),
            entry_of("assistant", "two"),
            entry_of("user", "three"),
        ]);

        // When rendering it.
        let html = render_html(&doc);

        // Then no contents listing is emitted.
        assert!(!html.contains("Contents"), "output still has a TOC");
        assert!(!html.contains("class=\"toc\""));
        // And nothing links anywhere, since there is nothing to link to.
        assert!(!html.contains("href=\"#"));
    }

    #[rstest::rstest]
    fn document_contains_no_script_element() {
        // Given a document whose body mentions script.
        let doc = doc_with(vec![entry_of("assistant", "no scripts here")]);

        // When rendering it.
        let html = render_html(&doc);

        // Then the file carries no script element or javascript url.
        assert!(!html.contains("<script"));
        assert!(!html.contains("javascript:"));
    }
}
