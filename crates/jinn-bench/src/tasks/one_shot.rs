//! 1-shot bench tasks — single message, model produces output from scratch.

use crate::task::BenchTask;

mod hello_world;
mod http_server;
mod json_parser;
mod markdown_to_html;
mod word_frequency;

#[cfg(test)]
mod noop;

/// Returns all 1-shot bench tasks.
#[cfg_attr(not(test), expect(unused_mut, reason = "mutable only in test builds for noop push"))]
pub fn tasks() -> Vec<BenchTask> {
    let mut list = vec![
        hello_world::task(),
        json_parser::task(),
        word_frequency::task(),
        http_server::task(),
        markdown_to_html::task(),
    ];
    #[cfg(test)]
    list.push(noop::task());
    list
}
