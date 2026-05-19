//! Token expansion — replaces `#name` tokens using templates from the store.

use super::PromptTemplateStore;

/// Expands `#name` tokens in `text` using templates from the store.
///
/// A valid token is a `#` followed by one or more non-whitespace, non-`#`
/// characters. The `#` must be at the start of the string or preceded by a
/// space. On exact name match, `#name` is replaced with the template body.
/// Unknown names are left as literal `#name` text.
///
/// This is a pure function with no side effects.
#[must_use]
pub fn expand_tokens(text: &str, store: &PromptTemplateStore) -> String {
    use unicode_segmentation::UnicodeSegmentation as _;

    let mut result = String::with_capacity(text.len());
    let graphemes: Vec<&str> = text.graphemes(true).collect();
    let len = graphemes.len();
    let mut i = 0;

    while i < len {
        let is_hash = graphemes.get(i) == Some(&"#");
        let preceded_by_boundary = i == 0 || graphemes.get(i.wrapping_sub(1)) == Some(&" ");
        let next_grapheme = graphemes.get(i + 1);
        let has_valid_name_start =
            next_grapheme.is_some() && next_grapheme != Some(&" ") && next_grapheme != Some(&"#");

        if is_hash && preceded_by_boundary && has_valid_name_start {
            let name_start = i + 1;
            let mut name_end = name_start;
            while name_end < len {
                let g = graphemes.get(name_end);
                if g.is_none() || g.is_some_and(|c| c.trim().is_empty() || *c == "#") {
                    break;
                }
                name_end += 1;
            }
            let name: String = graphemes
                .get(name_start..name_end)
                .map(|s| s.join(""))
                .unwrap_or_default();
            if let Some(template) = store.find_by_name(&name) {
                result.push_str(&template.body);
            } else {
                result.push('#');
                result.push_str(&name);
            }
            i = name_end;
        } else if let Some(&grapheme) = graphemes.get(i) {
            result.push_str(grapheme);
            i += 1;
        } else {
            i += 1;
        }
    }

    result
}

#[cfg(test)]
mod expand_tokens_tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::protocol::PromptTemplate;

    fn make_store(templates: Vec<(&str, &str, &str)>) -> PromptTemplateStore {
        PromptTemplateStore::from_vec(
            templates
                .into_iter()
                .map(|(name, description, body)| PromptTemplate {
                    name: name.to_owned(),
                    description: description.to_owned(),
                    body: body.to_owned(),
                })
                .collect(),
        )
    }

    #[rstest::rstest]
    fn expand_tokens_replaces_known_name() {
        // Given a store with a "greeting" template.
        let store = make_store(vec![("greeting", "A greeting", "Hello, world!")]);

        // When expanding tokens in "#greeting".
        let result = expand_tokens("#greeting", &store);

        // Then the token is replaced with the template body.
        assert_eq!(result, "Hello, world!");
    }

    #[rstest::rstest]
    fn expand_tokens_leaves_unknown_name() {
        // Given an empty store.
        let store = make_store(vec![]);

        // When expanding tokens in "#unknown".
        let result = expand_tokens("#unknown", &store);

        // Then the token is left as-is.
        assert_eq!(result, "#unknown");
    }

    #[rstest::rstest]
    fn expand_tokens_multiple_tokens() {
        // Given a store with two templates.
        let store = make_store(vec![("a", "First", "AAA"), ("b", "Second", "BBB")]);

        // When expanding "#a and #b".
        let result = expand_tokens("#a and #b", &store);

        // Then both tokens are replaced.
        assert_eq!(result, "AAA and BBB");
    }

    #[rstest::rstest]
    fn expand_tokens_at_start() {
        // Given a store with a "hi" template.
        let store = make_store(vec![("hi", "Greeting", "Hello")]);

        // When expanding "#hi there".
        let result = expand_tokens("#hi there", &store);

        // Then only the start token is replaced.
        assert_eq!(result, "Hello there");
    }

    #[rstest::rstest]
    fn expand_tokens_in_middle() {
        // Given a store with a "name" template.
        let store = make_store(vec![("name", "Name", "Alice")]);

        // When expanding "hello #name bye".
        let result = expand_tokens("hello #name bye", &store);

        // Then the middle token is replaced.
        assert_eq!(result, "hello Alice bye");
    }

    #[rstest::rstest]
    fn expand_tokens_at_end() {
        // Given a store with an "end" template.
        let store = make_store(vec![("end", "Ending", "DONE")]);

        // When expanding "start #end".
        let result = expand_tokens("start #end", &store);

        // Then the end token is replaced.
        assert_eq!(result, "start DONE");
    }

    #[rstest::rstest]
    fn expand_tokens_adjacent_tokens() {
        // Given a store with "a" and "b" templates.
        let store = make_store(vec![("a", "A", "X"), ("b", "B", "Y")]);

        // When expanding "#a #b".
        let result = expand_tokens("#a #b", &store);

        // Then both tokens are replaced.
        assert_eq!(result, "X Y");
    }

    #[rstest::rstest]
    fn expand_tokens_empty_body() {
        // Given a store with a template that has an empty body.
        let store = make_store(vec![("empty", "Empty", "")]);

        // When expanding "#empty".
        let result = expand_tokens("before #empty after", &store);

        // Then the token is replaced with nothing.
        assert_eq!(result, "before  after");
    }

    #[rstest::rstest]
    fn expand_tokens_midword_hash_ignored() {
        // Given a store with a "foo" template.
        let store = make_store(vec![("foo", "Foo", "BAR")]);

        // When expanding "abc#foo".
        let result = expand_tokens("abc#foo", &store);

        // Then the midword hash is left as-is.
        assert_eq!(result, "abc#foo");
    }

    #[rstest::rstest]
    fn expand_tokens_hash_at_end_of_string() {
        // Given a store with templates.
        let store = make_store(vec![("foo", "Foo", "BAR")]);

        // When expanding a trailing "#".
        let result = expand_tokens("test#", &store);

        // Then it's left as-is.
        assert_eq!(result, "test#");
    }

    #[rstest::rstest]
    fn expand_tokens_bare_hash() {
        // Given a store with templates.
        let store = make_store(vec![]);

        // When expanding a lone "#".
        let result = expand_tokens("#", &store);

        // Then it's left as-is.
        assert_eq!(result, "#");
    }

    #[rstest::rstest]
    fn expand_tokens_double_hash_passthrough() {
        // Given a store with templates.
        let store = make_store(vec![("foo", "Foo", "BAR")]);

        // When expanding "##foo".
        let result = expand_tokens("##foo", &store);

        // Then the first # triggers expansion of "#foo" (second # is not a valid
        // name start since next char is #).
        // Actually: chars[0]='#', chars[1]='#' → has_valid_name_start is false
        // because next_char is Some('#'). So first # is literal, then second #
        // at i=1 is preceded by non-space → midword → literal.
        assert_eq!(result, "##foo");
    }

    #[rstest::rstest]
    fn expand_tokens_empty_text() {
        // Given a store.
        let store = make_store(vec![]);

        // When expanding empty text.
        let result = expand_tokens("", &store);

        // Then result is empty.
        assert_eq!(result, "");
    }
}
