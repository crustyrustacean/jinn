//! 1-shot bench tasks — single message, model produces output from scratch.

use crate::task::BenchTask;

mod hello_world;
mod http_server;
mod json_parser;
mod markdown_to_html;
mod word_frequency;

/// Returns all 1-shot bench tasks.
pub fn tasks() -> Vec<BenchTask> {
    vec![
        hello_world::task(),
        json_parser::task(),
        word_frequency::task(),
        http_server::task(),
        markdown_to_html::task(),
    ]
}
