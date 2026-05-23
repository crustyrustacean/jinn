//! Example workflow definitions.
//!
//! Contains built-in workflow graph builders that can be registered at startup
//! and triggered via `/workflow <name>`.

use nullslop_workflow::graph::{WorkflowGraph, WorkflowGraphBuilder};
use nullslop_workflow::port::PortDef;

use super::llm_node::LlmNode;

/// Builds the "research-extract-summarize" workflow graph.
///
/// A 3-node pipeline:
/// 1. **research** — researches the given topic thoroughly
/// 2. **extract** — extracts key facts from the research
/// 3. **summarize** — writes a concise executive summary
///
/// # Errors
///
/// Returns an error if the graph fails validation (should not happen with static definitions).
#[expect(
    clippy::expect_used,
    reason = "static graph definition should always be valid"
)]
pub fn build_research_extract_summarize() -> WorkflowGraph {
    let research = LlmNode::new(
        "You are a research assistant. Research the given topic thoroughly and provide detailed findings with sources.",
    );
    let extract = LlmNode::new(
        "You are a data extraction specialist. Given research findings, extract the key facts, figures, and insights as a structured bullet-point list.",
    );
    let summarize = LlmNode::new(
        "You are a summary writer. Given extracted key facts, write a concise summary suitable for a busy executive in 2-3 paragraphs.",
    );

    let mut builder = WorkflowGraphBuilder::new();
    builder.add_node("research".to_owned(), Box::new(research));
    builder.add_node("extract".to_owned(), Box::new(extract));
    builder.add_node("summarize".to_owned(), Box::new(summarize));
    builder
        .connect("research", "response", "extract", "prompt")
        .expect("research → extract connection should be valid");
    builder
        .connect("extract", "response", "summarize", "prompt")
        .expect("extract → summarize connection should be valid");
    builder
        .build()
        .expect("research-extract-summarize graph should be valid")
}
