//! Token expansion - replaces `#name` tokens using templates from the store.

use std::sync::LazyLock;

use regex::Regex;

use super::PromptTemplateStore;

static TOKEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(^|[ \n])#([^\s#]+)").expect("valid token regex"));

/// Expands `#name` tokens in `text` using templates from the store.
///
/// A valid token is a `#` preceded by start-of-string, space, or newline,
/// followed by one or more non-whitespace, non-`#` characters. On exact name
/// match, `#name` is replaced with the template body. Unknown names are left
/// as literal `#name` text.
///
/// This is a pure function with no side effects.
#[must_use]
pub fn expand_tokens(text: &str, store: &PromptTemplateStore) -> String {
    TOKEN_RE
        .replace_all(text, |caps: &regex::Captures<'_>| {
            let boundary = &caps[1];
            let name = &caps[2];
            if let Some(template) = store.find_by_name(name) {
                format!("{boundary}{}", template.body)
            } else {
                caps[0].to_owned()
            }
        })
        .into_owned()
}

#[cfg(test)]
mod expand_tokens_tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unreachable, clippy::indexing_slicing, reason = "test code")]
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

    #[rstest::rstest]
    fn expand_tokens_after_newline() {
        // Given a store with a "greeting" template.
        let store = make_store(vec![("greeting", "A greeting", "Hello!")]);

        // When expanding "foo\n#greeting".
        let result = expand_tokens("foo\n#greeting", &store);

        // Then the token after newline is replaced.
        assert_eq!(result, "foo\nHello!");
    }

    #[rstest::rstest]
    fn expand_tokens_after_newline_unknown_left_as_is() {
        // Given a store with no matching templates.
        let store = make_store(vec![]);

        // When expanding "foo\n#unknown".
        let result = expand_tokens("foo\n#unknown", &store);

        // Then the token is left as-is.
        assert_eq!(result, "foo\n#unknown");
    }

    #[rstest::rstest]
    fn expand_tokens_tab_not_a_boundary() {
        // Given a store with a "foo" template.
        let store = make_store(vec![("foo", "Foo", "BAR")]);

        // When expanding "foo\t#foo".
        let result = expand_tokens("foo\t#foo", &store);

        // Then the token after tab is NOT expanded (tab is not a boundary).
        assert_eq!(result, "foo\t#foo");
    }
}
