//! Tree-sitter AST utilities for benchmark verification.
//!
//! Provides language-specific helpers for parsing source code and querying
//! structural elements (classes, functions, methods). Each language has its
//! own submodule with convenience wrappers over the tree-sitter API.

pub mod python;
