//! Python-specific Tree-sitter helpers for AST verification.
//!
//! Provides functions to parse Python source code and query structural
//! elements like classes, methods, and top-level functions.

#![allow(
    clippy::missing_docs_in_private_items,
    reason = "internal AST query helpers"
)]

use tree_sitter::Parser;

/// Parses Python source code and returns the syntax tree.
///
/// Returns `None` if the parser fails to produce a tree (e.g., out of memory).
/// Note that tree-sitter is error-tolerant: even malformed input produces a
/// partial tree with `ERROR` nodes.
///
/// # Panics
///
/// Panics if the Python language grammar fails to load (should never happen
/// with a correctly compiled `tree-sitter-python` crate).
#[expect(clippy::expect_used, reason = "language grammar is statically available")]
pub fn parse(source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .expect("failed to set Python language");
    parser.parse(source, None)
}

/// Finds a class definition by name among the direct children of a node.
///
/// Typically called on the root node to find top-level classes. Returns `None`
/// if no class with the given name exists.
pub fn find_class<'a>(
    node: &'a tree_sitter::Node<'a>,
    source: &[u8],
    name: &str,
) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "class_definition"
            && let Some(name_node) = child.child_by_field_name("name")
            && let Ok(text) = name_node.utf8_text(source)
            && text == name
        {
            return Some(child);
        }
    }
    None
}

/// Finds a function definition by name among the direct children of a node.
///
/// Typically called on the root node to find top-level functions. Returns
/// `None` if no function with the given name exists.
pub fn find_function<'a>(
    node: &'a tree_sitter::Node<'a>,
    source: &[u8],
    name: &str,
) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_definition"
            && let Some(name_node) = child.child_by_field_name("name")
            && let Ok(text) = name_node.utf8_text(source)
            && text == name
        {
            return Some(child);
        }
    }
    None
}

/// Returns the names of all methods (function definitions) in a class body.
///
/// Only collects `function_definition` nodes that are direct children of the
/// class body. Docstrings (expression statements) and assignments are ignored.
pub fn method_names(class_node: &tree_sitter::Node<'_>, source: &[u8]) -> Vec<String> {
    let Some(body) = class_node.child_by_field_name("body") else {
        return Vec::new();
    };

    let mut cursor = body.walk();
    body.children(&mut cursor)
        .filter(|child| child.kind() == "function_definition")
        .filter_map(|child| {
            child
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(String::from)
        })
        .collect()
}

/// Returns the names of all top-level function definitions in a module.
///
/// Only collects `function_definition` nodes that are direct children of the
/// root module node.
pub fn find_top_level_functions(
    root: &tree_sitter::Node<'_>,
    source: &[u8],
) -> Vec<String> {
    let mut cursor = root.walk();
    root.children(&mut cursor)
        .filter(|child| child.kind() == "function_definition")
        .filter_map(|child| {
            child
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(String::from)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]

    use super::*;

    #[test]
    fn parse_valid_python_returns_module_root() {
        // Given valid Python source.
        let source = "x = 1\n";

        // When parsing.
        let tree = parse(source).expect("parse should succeed");

        // Then the root node is a module.
        assert_eq!(tree.root_node().kind(), "module");
    }

    #[test]
    fn find_class_returns_node_when_present() {
        // Given Python source with a class named "Foo".
        let source = "class Foo:\n    pass\n";
        let tree = parse(source).expect("parse should succeed");
        let root = tree.root_node();
        let bytes = source.as_bytes();

        // When finding class "Foo".
        let result = find_class(&root, bytes, "Foo");

        // Then the class node is found.
        assert!(result.is_some());
        assert_eq!(result.expect("node").kind(), "class_definition");
    }

    #[test]
    fn find_class_returns_none_when_absent() {
        // Given Python source without a class.
        let source = "x = 1\n";
        let tree = parse(source).expect("parse should succeed");
        let root = tree.root_node();
        let bytes = source.as_bytes();

        // When finding class "Foo".
        let result = find_class(&root, bytes, "Foo");

        // Then no class is found.
        assert!(result.is_none());
    }

    #[test]
    fn find_function_returns_node_when_present() {
        // Given Python source with a function named "main".
        let source = "def main():\n    pass\n";
        let tree = parse(source).expect("parse should succeed");
        let root = tree.root_node();
        let bytes = source.as_bytes();

        // When finding function "main".
        let result = find_function(&root, bytes, "main");

        // Then the function node is found.
        assert!(result.is_some());
        assert_eq!(result.expect("node").kind(), "function_definition");
    }

    #[test]
    fn find_function_returns_none_when_absent() {
        // Given Python source without a function.
        let source = "x = 1\n";
        let tree = parse(source).expect("parse should succeed");
        let root = tree.root_node();
        let bytes = source.as_bytes();

        // When finding function "main".
        let result = find_function(&root, bytes, "main");

        // Then no function is found.
        assert!(result.is_none());
    }

    #[test]
    fn method_names_returns_all_methods() {
        // Given a class with multiple methods.
        let source = "\
class Processor:
    def load(self, data):
        self.data = data
    def process(self):
        return self.data
";
        let tree = parse(source).expect("parse should succeed");
        let root = tree.root_node();
        let bytes = source.as_bytes();
        let class = find_class(&root, bytes, "Processor").expect("class");

        // When collecting method names.
        let names = method_names(&class, bytes);

        // Then both methods are found.
        assert_eq!(names, vec!["load", "process"]);
    }

    #[test]
    fn method_names_ignores_docstrings() {
        // Given a class where methods have docstrings.
        let source = "\
class Foo:
    \"\"\"Class docstring.\"\"\"
    def bar(self):
        \"\"\"Method docstring.\"\"\"
        return 1
";
        let tree = parse(source).expect("parse should succeed");
        let root = tree.root_node();
        let bytes = source.as_bytes();
        let class = find_class(&root, bytes, "Foo").expect("class");

        // When collecting method names.
        let names = method_names(&class, bytes);

        // Then only "bar" is found (docstrings are expression statements, not function definitions).
        assert_eq!(names, vec!["bar"]);
    }

    #[test]
    fn extra_comments_and_blank_lines_dont_affect_structure() {
        // Given Python source with comments and blank lines.
        let source = "\
# A comment at the top

class NumberProcessor:
    \"\"\"A docstring.\"\"\"

    # Another comment
    def load(self, data):
        self.data = data

    def sum(self):
        return sum(self.data)

# Bottom comment

def main():
    pass
";
        let tree = parse(source).expect("parse should succeed");
        let root = tree.root_node();
        let bytes = source.as_bytes();

        // When finding the class and its methods.
        let class = find_class(&root, bytes, "NumberProcessor");
        assert!(class.is_some());
        let names = method_names(&class.expect("class"), bytes);

        // Then methods are correctly detected despite comments and blanks.
        assert_eq!(names, vec!["load", "sum"]);

        // And the top-level function is found.
        let funcs = find_top_level_functions(&root, bytes);
        assert_eq!(funcs, vec!["main"]);
    }

    #[test]
    fn find_top_level_functions_returns_all_functions() {
        // Given Python source with multiple top-level functions.
        let source = "\
def foo():
    pass
def bar():
    pass
";
        let tree = parse(source).expect("parse should succeed");
        let root = tree.root_node();
        let bytes = source.as_bytes();

        // When finding top-level functions.
        let funcs = find_top_level_functions(&root, bytes);

        // Then both functions are found.
        assert_eq!(funcs, vec!["foo", "bar"]);
    }

    #[test]
    fn find_class_distinguishes_between_classes() {
        // Given Python source with two classes.
        let source = "\
class Foo:
    pass
class Bar:
    pass
";
        let tree = parse(source).expect("parse should succeed");
        let root = tree.root_node();
        let bytes = source.as_bytes();

        // When finding each class by name.
        assert!(find_class(&root, bytes, "Foo").is_some());
        assert!(find_class(&root, bytes, "Bar").is_some());
        assert!(find_class(&root, bytes, "Baz").is_none());
    }

    #[test]
    fn method_names_handles_dunder_methods() {
        // Given a class with dunder methods.
        let source = "\
class Foo:
    def __init__(self):
        pass
    def __str__(self):
        return 'foo'
";
        let tree = parse(source).expect("parse should succeed");
        let root = tree.root_node();
        let bytes = source.as_bytes();
        let class = find_class(&root, bytes, "Foo").expect("class");

        // When collecting method names.
        let names = method_names(&class, bytes);

        // Then dunder methods are included.
        assert_eq!(names, vec!["__init__", "__str__"]);
    }
}
