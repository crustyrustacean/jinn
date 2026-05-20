//! Command template parser — extracts positional and named parameters from shell commands.
//!
//! A [`CommandTemplate`] parses a command string like `script.sh $1 $2 $1` or
//! `./foo.sh <branch> <target>` and extracts the unique parameters in order of
//! first appearance. It can then:
//!
//! - **Render** the command with concrete arguments
//! - **Display** the command with human-readable tokens
//!
//! # Parameter syntax
//!
//! - `<name>` — named parameter (positional, filled by arg in same position)
//! - `$1` through `$9` — numeric positional parameters (backward compatibility)
//! - `$@` and `$*` — "all args" sentinel (accepts variable number of args)
//!
//! Parameters are deduplicated by identity. `$1 <foo> $1` produces `[Positional(1), Named("foo")]`
//! and substitutes both `$1` occurrences with the same arg.

use std::fmt;
use unicode_segmentation::UnicodeSegmentation;

/// A segment of a displayed command line, tagged with its parameter index.
///
/// Used by [`CommandTemplate::display_line_segments`] to produce structured
/// output suitable for styled rendering. Static text has `param_index = None`;
/// parameter placeholders and their substituted values have `Some(idx)` where
/// `idx` is the index into the template's params list.
///
/// This design enables future per-argument color schemes (gradient, rainbow)
/// by only changing the color-assignment logic in the renderer — the data
/// structure already knows which arg each segment belongs to.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplaySegment {
    /// The text to display (placeholder like `<branch>` or substituted value).
    pub text: String,
    /// Index into the template's `params` list, or `None` for static text.
    pub param_index: Option<usize>,
}

impl DisplaySegment {
    /// Creates a static (non-parameter) segment.
    fn static_text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            param_index: None,
        }
    }

    /// Creates a parameter segment with the given param index.
    fn param(text: impl Into<String>, index: usize) -> Self {
        Self {
            text: text.into(),
            param_index: Some(index),
        }
    }
}

/// A single parameter extracted from a command template.
///
/// Parameters are deduplicated — each unique token appears at most once.
/// During rendering, *all* occurrences (including duplicates) in the source
/// string are replaced with the corresponding argument value.
#[derive(Debug, Clone, PartialEq)]
pub enum Param {
    /// A named parameter like `<foo>` — filled by positional args.
    Named(String),
    /// A numeric positional parameter like `$1`.
    Positional(usize),
    /// The "all args" splat (`$@`, `$*`).
    Splat,
}

impl fmt::Display for Param {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(name) => write!(f, "<{name}>"),
            Self::Positional(n) => write!(f, "${n}"),
            Self::Splat => write!(f, "$@"),
        }
    }
}

/// A parsed shell command template with extracted parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandTemplate {
    /// The original command string.
    source: String,
    /// Unique parameters in order of first appearance.
    /// E.g., `script.sh $1 <branch> $@` → `[Positional(1), Named("branch"), Splat]`.
    params: Vec<Param>,
}

/// Try to parse a `$N`, `$@`, or `$*` token at position `i` in `graphemes`.
///
/// Returns `Some((Param, graphemes_consumed))` on success, `None` if position `i`
/// is not a recognized dollar token.
fn try_parse_dollar(graphemes: &[&str], i: usize) -> Option<(Param, usize)> {
    if graphemes[i] != "$" || i + 1 >= graphemes.len() {
        return None;
    }
    let next = graphemes[i + 1];
    if next.len() == 1 && next.as_bytes()[0].is_ascii_digit() && next != "0" {
        let n = (next.as_bytes()[0] - b'0') as usize;
        Some((Param::Positional(n), 2))
    } else if next == "@" || next == "*" {
        Some((Param::Splat, 2))
    } else {
        None
    }
}

/// Try to parse a `<name>` token at position `i` in `graphemes`.
///
/// Returns `Some((Param::Named(name), graphemes_consumed))` if a well-formed
/// `<name>` token is found (non-empty name, closing `>` present).
/// Returns `None` otherwise.
fn try_parse_named(graphemes: &[&str], i: usize) -> Option<(Param, usize)> {
    if graphemes[i] != "<" {
        return None;
    }
    let start = i + 1;
    let mut end = start;
    while end < graphemes.len() && graphemes[end] != ">" {
        end += 1;
    }
    if end > start && end < graphemes.len() {
        let name: String = graphemes[start..end].join("");
        Some((Param::Named(name), end - i + 1))
    } else {
        None
    }
}

impl CommandTemplate {
    /// Parse a command string and extract parameters.
    ///
    /// Recognizes three token types:
    /// - `<name>` — a named parameter
    /// - `$1`–`$9` — a numeric positional parameter
    /// - `$@` / `$*` — the "all args" splat
    ///
    /// Parameters are deduplicated: if the same token appears multiple times,
    /// only the first occurrence is recorded. The order of first appearance
    /// defines the parameter order for arg assignment.
    #[must_use]
    pub fn parse(command: &str) -> Self {
        let mut params: Vec<Param> = Vec::new();
        let graphemes: Vec<&str> = command.graphemes(true).collect();
        let mut i = 0;

        while i < graphemes.len() {
            if let Some((param, consumed)) = try_parse_dollar(&graphemes, i)
                .or_else(|| try_parse_named(&graphemes, i))
            {
                if !params.contains(&param) {
                    params.push(param);
                }
                i += consumed;
                continue;
            }
            i += 1;
        }

        Self {
            source: command.to_owned(),
            params,
        }
    }

    /// Whether this template requires any arguments.
    pub fn has_params(&self) -> bool {
        !self.params.is_empty()
    }

    /// The number of non-splat parameters.
    ///
    /// For `$1 $2 $@` this returns 2 — the number of positional-or-named slots
    /// that consume one argument each. Splat consumes all remaining args.
    #[must_use]
    pub fn param_count(&self) -> usize {
        self.params
            .iter()
            .filter(|p| !matches!(p, Param::Splat))
            .count()
    }

    /// Whether `$@` or `$*` was found in the command.
    #[must_use]
    pub fn has_splat(&self) -> bool {
        self.params.iter().any(|p| matches!(p, Param::Splat))
    }

    /// The unique parameters in order of first appearance.
    #[must_use]
    pub fn params(&self) -> &[Param] {
        &self.params
    }

    /// Render the command with concrete arguments substituted.
    ///
    /// Args are assigned positionally: `params[0]` → `args[0]`, `params[1]` → `args[1]`, etc.
    /// Splat (`$@` / `$*`) is replaced with all args joined by spaces.
    /// Named params (`<name>`) receive the positional arg at their index.
    ///
    /// # Panics
    ///
    /// Panics if there aren't enough args for the non-splat params.
    pub fn render(&self, args: &[String]) -> String {
        let mut result = self.source.clone();

        // Build a map: for each non-splat param, which arg index it uses.
        // Splat is handled separately.

        // Replace non-splat params in order, assigning args sequentially.
        // Each non-splat param gets the next available arg (args[0], args[1], ...).
        // If an arg is missing, substitute empty string instead of panicking.
        let mut arg_idx = 0usize;
        for param in &self.params {
            match param {
                Param::Named(name) => {
                    let search = format!("<{name}>");
                    let replacement = if arg_idx < args.len() {
                        shell_quote(&args[arg_idx])
                    } else {
                        String::new()
                    };
                    result = result.replace(&search, &replacement);
                    arg_idx += 1;
                }
                Param::Positional(_n) => {
                    let search = format!("{param}");
                    let replacement = if arg_idx < args.len() {
                        shell_quote(&args[arg_idx])
                    } else {
                        String::new()
                    };
                    result = result.replace(&search, &replacement);
                    arg_idx += 1;
                }
                Param::Splat => {
                    // Handled below after non-slot args are consumed.
                }
            }
        }

        // Replace $@ and $* with remaining args (those not consumed by non-splat params).
        if self.has_splat() {
            let joined = args[arg_idx..]
                .iter()
                .map(|a| shell_quote(a))
                .collect::<Vec<_>>()
                .join(" ");
            result = result.replace("$@", &joined);
            result = result.replace("$*", &joined);
        }

        result
    }

    /// Render the command with display tokens.
    ///
    /// - `<name>` stays as `<name>` (already display-ready)
    /// - `$1` → `<1>`, `$2` → `<2>` etc.
    /// - `$@` / `$*` → `<args>`
    ///
    /// Used in the arg-input popup UI.
    #[must_use]
    pub fn display(&self) -> String {
        let mut result = self.source.clone();

        // Replace in reverse order by source occurrence to avoid offset issues.
        // Since we're doing string.replace, order doesn't matter for correctness,
        // but we do non-splat first, then splat.
        for param in self.params.iter().rev() {
            match param {
                Param::Named(_name) => {
                    // <name> is already display-ready, no conversion needed.
                }
                Param::Positional(n) => {
                    let search = format!("${n}");
                    result = result.replace(&search, &format!("<{n}>"));
                }
                Param::Splat => {
                    result = result.replace("$@", "<args>");
                    result = result.replace("$*", "<args>");
                }
            }
        }

        result
    }

    /// Produce structured display lines with parameter substitution for the arg-input popup.
    ///
    /// Splits the display form of the command on ` && `, producing one
    /// [`Vec<DisplaySegment>`] per line. Each segment is tagged with its parameter
    /// index (or `None` for static text). When a user-provided arg is available
    /// for a parameter, the placeholder is replaced with the arg value.
    ///
    /// The first line is bare; subsequent lines are prefixed with `&& ` in a static
    /// segment. Non-last lines are suffixed with ` \` in a static segment.
    ///
    /// This is render-time only — it does not affect command execution.
    #[must_use]
    pub fn display_line_segments(&self, args: &[String]) -> Vec<Vec<DisplaySegment>> {
        // Build the display form of the command (same logic as display()).
        let display_str = self.display();

        // Split on " && " to get the raw segments.
        let raw_parts: Vec<&str> = display_str.split(" && ").collect();

        // For each raw part, tokenize into DisplaySegments.
        let mut lines = Vec::with_capacity(raw_parts.len());
        for (line_idx, raw) in raw_parts.iter().enumerate() {
            let mut segments = Vec::new();

            // Prefix for non-first lines.
            if line_idx > 0 {
                segments.push(DisplaySegment::static_text("  && "));
            }

            // Parse the raw text for <...> placeholders.
            segments.extend(self.tokenize_display_line(raw, args));

            // Suffix for non-last lines.
            if line_idx < raw_parts.len() - 1 {
                segments.push(DisplaySegment::static_text(" \\"));
            }

            lines.push(segments);
        }

        lines
    }

    /// Tokenize a display-form line into `DisplaySegment`s.
    ///
    /// Walks the text looking for `<...>` patterns. For each match, looks up
    /// the corresponding param index. If a user-provided arg is available,
    /// substitutes it; otherwise keeps the placeholder text.
    fn tokenize_display_line(&self, line: &str, args: &[String]) -> Vec<DisplaySegment> {
        let mut segments = Vec::new();
        let graphemes: Vec<&str> = line.graphemes(true).collect();
        let mut i = 0;
        let mut static_start = 0;

        // Build a lookup: display token -> param index.
        // E.g., Named("branch") -> ("<branch>", param_list_index).
        //        Positional(1) -> ("<1>", param_list_index).
        //        Splat -> ("<args>", param_list_index).
        let mut arg_offset = 0usize;
        let mut token_map: Vec<(String, usize, usize)> = Vec::new(); // (display_token, param_index, arg_offset_for_this_param)
        for (pidx, param) in self.params.iter().enumerate() {
            match param {
                Param::Named(name) => {
                    token_map.push((format!("<{name}>"), pidx, arg_offset));
                    arg_offset += 1;
                }
                Param::Positional(n) => {
                    token_map.push((format!("<{n}>"), pidx, arg_offset));
                    arg_offset += 1;
                }
                Param::Splat => {
                    token_map.push(("<args>".to_owned(), pidx, arg_offset));
                }
            }
        }

        while i < graphemes.len() {
            if graphemes[i] == "<" {
                // Potential placeholder — scan for '>'.
                let start = i + 1;
                let mut end = start;
                while end < graphemes.len() && graphemes[end] != ">" {
                    end += 1;
                }

                if end < graphemes.len() && end > start {
                    let token_text: String = graphemes[start..end].join("");
                    let full_token = format!("<{token_text}>");

                    // Check if this matches any known param.
                    if let Some((_, param_idx, arg_off)) =
                        token_map.iter().find(|(tok, _, _)| *tok == full_token)
                    {
                        // Emit preceding static text.
                        if static_start < i {
                            let text: String = graphemes[static_start..i].join("");
                            segments.push(DisplaySegment::static_text(text));
                        }

                        // Determine display text: substitute arg if available.
                        let display_text = if *arg_off < args.len() {
                            match &self.params[*param_idx] {
                                Param::Splat => args[*arg_off..].join(" "),
                                _ => args[*arg_off].clone(),
                            }
                        } else {
                            full_token.clone()
                        };

                        segments.push(DisplaySegment::param(display_text, *param_idx));
                        static_start = end + 1;
                        i = end + 1;
                        continue;
                    }
                }
            }
            i += 1;
        }

        // Emit remaining static text.
        if static_start < graphemes.len() {
            let text: String = graphemes[static_start..].join("");
            if !text.is_empty() {
                segments.push(DisplaySegment::static_text(text));
            }
        }

        segments
    }
}

/// Shell-quote a value for safe interpolation into a `$SHELL -c` command.
///
/// Uses single-quote wrapping with `\'\'\'` escape for embedded single quotes.
/// Only applies quoting when the value contains spaces or shell-special characters.
/// Safe values pass through unchanged.
#[must_use]
pub fn shell_quote(s: &str) -> String {
    /// Characters that require shell quoting.
    const SHELL_SPECIAL: &[char] = &[
        ' ', '\t', '\n', '\r', '|', '&', ';', '<', '>', '$', '`', '\\', '"', '\'', '(', ')', '*',
        '?', '[', ']', '~', '#', '!', '{', '}', '=', ':',
    ];

    if s.is_empty() {
        return "''".to_owned();
    }

    if !s.contains(SHELL_SPECIAL) {
        return s.to_owned();
    }

    // Wrap in single quotes, escaping embedded single quotes as '\'\''.
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// Parse a user input string into arguments, preserving quote characters.
///
/// Same splitting logic as [`parse_quoted_args`] (splits on unquoted whitespace,
/// respects backslash escapes) but keeps the double-quote characters in the tokens.
/// Used for display purposes so users see their literal input including quotes.
#[must_use]
pub fn split_preserving_quotes(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let graphemes: Vec<&str> = input.graphemes(true).collect();
    let mut i = 0;

    while i < graphemes.len() {
        let g = graphemes[i];
        if in_quotes {
            if g == "\\" {
                // Backslash escape inside quotes: next grapheme is literal (skip backslash).
                current.push('\\');
                i += 1;
                if i < graphemes.len() {
                    current.push_str(graphemes[i]);
                }
            } else if g == "\"" {
                // End of quoted section — keep the quote char.
                current.push('"');
                in_quotes = false;
            } else {
                current.push_str(g);
            }
        } else if g == "\\" {
            // Backslash escape outside quotes.
            current.push('\\');
            i += 1;
            if i < graphemes.len() {
                current.push_str(graphemes[i]);
            }
        } else if g == "\"" {
            // Start of quoted section — keep the quote char.
            current.push('"');
            in_quotes = true;
        } else if g.chars().next().is_some_and(char::is_whitespace) {
            if !current.is_empty() {
                args.push(current.clone());
                current.clear();
            }
        } else {
            current.push_str(g);
        }
        i += 1;
    }

    if !current.is_empty() {
        args.push(current);
    }

    args
}

/// Parse a user input string into arguments, respecting double quotes and backslash escapes.
///
/// Rules:
/// - Outside quotes, whitespace separates tokens.
/// - Inside `"..."`, everything (including spaces) is one token; the quotes are stripped.
/// - Backslash escapes: `\"` → `"`, `\\` → `\`, `\x` → `x` for any other char.
/// - An unterminated quote treats the remaining input as the content of the quote.
///
/// # Examples
///
/// ```text
/// foo bar        → ["foo", "bar"]
/// "foo bar"      → ["foo bar"]
/// a "b c" d      → ["a", "b c", "d"]
/// foo\"bar       → ["foo\"bar"]
/// ```
#[must_use]
pub fn parse_quoted_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let graphemes: Vec<&str> = input.graphemes(true).collect();
    let mut i = 0;

    while i < graphemes.len() {
        let g = graphemes[i];
        if in_quotes {
            if g == "\\" {
                // Backslash escape inside quotes: next grapheme is literal.
                i += 1;
                if i < graphemes.len() {
                    current.push_str(graphemes[i]);
                } else {
                    // Trailing backslash — treat as literal.
                    current.push('\\');
                }
            } else if g == "\"" {
                // End of quoted section.
                in_quotes = false;
            } else {
                current.push_str(g);
            }
        } else if g == "\\" {
            // Backslash escape outside quotes.
            i += 1;
            if i < graphemes.len() {
                current.push_str(graphemes[i]);
            } else {
                current.push('\\');
            }
        } else if g == "\"" {
            in_quotes = true;
        } else if g.chars().next().is_some_and(char::is_whitespace) {
            if !current.is_empty() {
                args.push(current.clone());
                current.clear();
            }
        } else {
            current.push_str(g);
        }
        i += 1;
    }

    if !current.is_empty() {
        args.push(current);
    }

    args
}

impl fmt::Display for CommandTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;

    // --- Parsing: $N syntax (backward compat) ---

    #[rstest::rstest]
    fn parse_no_params() {
        let tmpl = CommandTemplate::parse("echo hello");
        assert!(!tmpl.has_params());
        assert!(tmpl.params().is_empty());
        assert_eq!(tmpl.param_count(), 0);
    }

    #[rstest::rstest]
    fn parse_one_param() {
        let tmpl = CommandTemplate::parse("script.sh $1");
        assert!(tmpl.has_params());
        assert_eq!(tmpl.params(), &[Param::Positional(1)]);
        assert_eq!(tmpl.param_count(), 1);
    }

    #[rstest::rstest]
    fn parse_multiple_params() {
        let tmpl = CommandTemplate::parse("script.sh $1 $2");
        assert_eq!(tmpl.params(), &[Param::Positional(1), Param::Positional(2)]);
        assert_eq!(tmpl.param_count(), 2);
    }

    #[rstest::rstest]
    fn parse_deduplicates_repeated_params() {
        let tmpl = CommandTemplate::parse("script.sh $1 $2 $1");
        assert_eq!(tmpl.params(), &[Param::Positional(1), Param::Positional(2)]);
        assert_eq!(tmpl.param_count(), 2);
    }

    #[rstest::rstest]
    fn parse_splat_at() {
        let tmpl = CommandTemplate::parse("script.sh $@");
        assert!(tmpl.has_splat());
        assert!(tmpl.has_params());
    }

    #[rstest::rstest]
    fn parse_splat_star() {
        let tmpl = CommandTemplate::parse("script.sh $*");
        assert!(tmpl.has_splat());
    }

    #[rstest::rstest]
    fn parse_mixed_numbered_and_splat() {
        let tmpl = CommandTemplate::parse("script.sh $1 $@");
        assert_eq!(tmpl.params(), &[Param::Positional(1), Param::Splat]);
        assert!(tmpl.has_splat());
        assert_eq!(tmpl.param_count(), 1);
    }

    #[rstest::rstest]
    fn parse_skips_dollar_zero() {
        let tmpl = CommandTemplate::parse("echo $0");
        assert!(!tmpl.has_params());
    }

    #[rstest::rstest]
    fn parse_non_consecutive_params() {
        let tmpl = CommandTemplate::parse("script.sh $1 $3");
        assert_eq!(tmpl.params(), &[Param::Positional(1), Param::Positional(3)]);
        assert_eq!(tmpl.param_count(), 2);
    }

    // --- Parsing: <name> syntax ---

    #[rstest::rstest]
    fn parse_named_param() {
        let tmpl = CommandTemplate::parse("script.sh <branch>");
        assert!(tmpl.has_params());
        assert_eq!(tmpl.params(), &[Param::Named("branch".to_owned())]);
        assert_eq!(tmpl.param_count(), 1);
    }

    #[rstest::rstest]
    fn parse_multiple_named_params() {
        let tmpl = CommandTemplate::parse("script.sh <branch> <target>");
        assert_eq!(
            tmpl.params(),
            &[
                Param::Named("branch".to_owned()),
                Param::Named("target".to_owned()),
            ]
        );
        assert_eq!(tmpl.param_count(), 2);
    }

    #[rstest::rstest]
    fn parse_deduplicates_named_params() {
        let tmpl = CommandTemplate::parse("script.sh <branch> <target> <branch>");
        assert_eq!(
            tmpl.params(),
            &[
                Param::Named("branch".to_owned()),
                Param::Named("target".to_owned()),
            ]
        );
        assert_eq!(tmpl.param_count(), 2);
    }

    #[rstest::rstest]
    fn parse_mixed_named_and_positional() {
        let tmpl = CommandTemplate::parse("script.sh <branch> $1");
        assert_eq!(
            tmpl.params(),
            &[Param::Named("branch".to_owned()), Param::Positional(1)]
        );
        assert_eq!(tmpl.param_count(), 2);
    }

    #[rstest::rstest]
    fn parse_named_with_splat() {
        let tmpl = CommandTemplate::parse("script.sh <branch> $@");
        assert_eq!(
            tmpl.params(),
            &[Param::Named("branch".to_owned()), Param::Splat]
        );
        assert!(tmpl.has_splat());
        assert_eq!(tmpl.param_count(), 1);
    }

    #[rstest::rstest]
    fn parse_multiple_named_same_value() {
        let tmpl = CommandTemplate::parse("script.sh <foo> <bar> <foo>");
        assert_eq!(
            tmpl.params(),
            &[
                Param::Named("foo".to_owned()),
                Param::Named("bar".to_owned())
            ]
        );
        assert_eq!(tmpl.param_count(), 2);
    }

    // --- Rendering: $N syntax ---

    #[rstest::rstest]
    fn render_no_params() {
        let tmpl = CommandTemplate::parse("echo hello");
        assert_eq!(tmpl.render(&[]), "echo hello");
    }

    #[rstest::rstest]
    fn render_one_param() {
        let tmpl = CommandTemplate::parse("script.sh $1");
        assert_eq!(
            tmpl.render(&["my-branch".to_owned()]),
            "script.sh my-branch"
        );
    }

    #[rstest::rstest]
    fn render_multiple_params() {
        let tmpl = CommandTemplate::parse("script.sh $1 $2");
        assert_eq!(
            tmpl.render(&["foo".to_owned(), "bar".to_owned()]),
            "script.sh foo bar"
        );
    }

    #[rstest::rstest]
    fn render_repeated_param() {
        let tmpl = CommandTemplate::parse("script.sh $1 $2 $1");
        assert_eq!(
            tmpl.render(&["branch".to_owned(), "dir".to_owned()]),
            "script.sh branch dir branch"
        );
    }

    #[rstest::rstest]
    fn render_splat() {
        let tmpl = CommandTemplate::parse("script.sh $@");
        assert_eq!(
            tmpl.render(&["a".to_owned(), "b".to_owned(), "c".to_owned()]),
            "script.sh a b c"
        );
    }

    // --- Rendering: <name> syntax ---

    #[rstest::rstest]
    fn render_one_named_param() {
        let tmpl = CommandTemplate::parse("script.sh <branch>");
        assert_eq!(
            tmpl.render(&["my-feature".to_owned()]),
            "script.sh my-feature"
        );
    }

    #[rstest::rstest]
    fn render_multiple_named_params() {
        let tmpl = CommandTemplate::parse("script.sh <branch> <target>");
        assert_eq!(
            tmpl.render(&["my-feature".to_owned(), "/tmp/workdir".to_owned()]),
            "script.sh my-feature /tmp/workdir"
        );
    }

    #[rstest::rstest]
    fn render_named_with_splat() {
        let tmpl = CommandTemplate::parse("script.sh <branch> $@");
        assert_eq!(
            tmpl.render(&["my-feature".to_owned(), "a".to_owned(), "b".to_owned()]),
            "script.sh my-feature a b"
        );
    }

    #[rstest::rstest]
    fn render_repeated_named_param() {
        let tmpl = CommandTemplate::parse("script.sh <branch> $2 <branch>");
        assert_eq!(
            tmpl.render(&["my-feature".to_owned(), "other".to_owned()]),
            "script.sh my-feature other my-feature"
        );
    }

    #[rstest::rstest]
    fn render_mixed_named_and_positional() {
        let tmpl = CommandTemplate::parse("script.sh <branch> $1");
        assert_eq!(
            tmpl.render(&["my-feature".to_owned(), "dup".to_owned()]),
            // <branch> gets args[0]="my-feature", $1 gets args[1]="dup"
            "script.sh my-feature dup"
        );
    }

    // --- Display ---

    #[rstest::rstest]
    fn display_no_params() {
        let tmpl = CommandTemplate::parse("echo hello");
        assert_eq!(tmpl.display(), "echo hello");
    }

    #[rstest::rstest]
    fn display_with_params() {
        let tmpl = CommandTemplate::parse("script.sh $1 $2");
        assert_eq!(tmpl.display(), "script.sh <1> <2>");
    }

    #[rstest::rstest]
    fn display_with_splat() {
        let tmpl = CommandTemplate::parse("script.sh $@");
        assert_eq!(tmpl.display(), "script.sh <args>");
    }

    #[rstest::rstest]
    fn display_repeated_params() {
        let tmpl = CommandTemplate::parse("script.sh $1 $2 $1");
        assert_eq!(tmpl.display(), "script.sh <1> <2> <1>");
    }

    #[rstest::rstest]
    fn display_named_params() {
        let tmpl = CommandTemplate::parse("script.sh <branch> <target>");
        // Named params are already displayed as `<branch> <target>`.
        assert_eq!(tmpl.display(), "script.sh <branch> <target>");
    }

    #[rstest::rstest]
    fn display_named_with_positional() {
        let tmpl = CommandTemplate::parse("script.sh <branch> $1");
        assert_eq!(tmpl.display(), "script.sh <branch> <1>");
    }

    // --- Shell redirection safety ---

    #[rstest::rstest]
    fn display_does_not_confuse_redirection_with_params() {
        let tmpl = CommandTemplate::parse("echo $1 > output.txt");
        assert_eq!(tmpl.display(), "echo <1> > output.txt");
    }

    #[rstest::rstest]
    fn render_preserves_redirection() {
        let tmpl = CommandTemplate::parse("echo $1 > output.txt");
        assert_eq!(
            tmpl.render(&["hello".to_owned()]),
            "echo hello > output.txt"
        );
    }

    // --- Edge cases ---

    #[rstest::rstest]
    fn parse_unclosed_angle_bracket_is_not_a_param() {
        let tmpl = CommandTemplate::parse("script.sh <unclosed");
        assert!(!tmpl.has_params());
    }

    #[rstest::rstest]
    fn parse_empty_angle_bracket_is_not_a_param() {
        let tmpl = CommandTemplate::parse("script.sh <>");
        assert!(!tmpl.has_params());
    }

    // --- Display trait ---

    #[rstest::rstest]
    fn display_trait_delegates_to_display_method() {
        let tmpl = CommandTemplate::parse("script.sh $1 $2");
        assert_eq!(format!("{tmpl}"), tmpl.display());
    }

    // --- display_line_segments ---

    #[rstest::rstest]
    fn display_line_segments_no_params() {
        // Given a simple command with no params and no &&.
        let tmpl = CommandTemplate::parse("echo hello");

        // When getting display line segments with no args.
        let lines = tmpl.display_line_segments(&[]);

        // Then it produces a single line with one static segment.
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], vec![DisplaySegment::static_text("echo hello")]);
    }

    #[rstest::rstest]
    fn display_line_segments_no_params_single_arg_ignored() {
        // Given a command with no params.
        let tmpl = CommandTemplate::parse("echo hello");

        // When passing args (should be ignored since no placeholders).
        let lines = tmpl.display_line_segments(&["ignored".to_owned()]);

        // Then same as no args.
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], vec![DisplaySegment::static_text("echo hello")]);
    }

    #[rstest::rstest]
    fn display_line_segments_splits_on_and_and() {
        // Given a command with && and no params.
        let tmpl = CommandTemplate::parse("echo hello && echo world");

        // When getting display line segments.
        let lines = tmpl.display_line_segments(&[]);

        // Then it produces two lines with continuation markers.
        assert_eq!(lines.len(), 2);
        // Line 1: "echo hello \"
        assert_eq!(
            lines[0],
            vec![
                DisplaySegment::static_text("echo hello"),
                DisplaySegment::static_text(" \\"),
            ]
        );
        // Line 2: "&& echo world"
        assert_eq!(
            lines[1],
            vec![
                DisplaySegment::static_text("  && "),
                DisplaySegment::static_text("echo world"),
            ]
        );
    }

    #[rstest::rstest]
    fn display_line_segments_five_parts() {
        // Given a long command split into 5 parts by &&.
        let tmpl = CommandTemplate::parse(
            "mkdir <branch> && cd <branch> && fossil open ../nullslop.fossil && fossil commit -m 'Open <branch>' --branch <branch> --allow-empty && echo ./<branch>",
        );

        // When getting display line segments with no args.
        let lines = tmpl.display_line_segments(&[]);

        // Then it produces 5 lines.
        assert_eq!(lines.len(), 5);

        // First line: "mkdir <branch> \"
        assert!(lines[0].last().unwrap().text.ends_with('\\'));
        // Last line: "&& echo ./<branch>" (no trailing \).
        assert!(!lines[4].last().unwrap().text.ends_with('\\'));
        // Lines 2-4 start with "  && ".
        assert_eq!(lines[1][0].text, "  && ");
        assert_eq!(lines[2][0].text, "  && ");
        assert_eq!(lines[3][0].text, "  && ");
        assert_eq!(lines[4][0].text, "  && ");
    }

    #[rstest::rstest]
    fn display_line_segments_last_line_no_trailing_backslash() {
        // Given a two-part command.
        let tmpl = CommandTemplate::parse("echo hello && echo world");

        // When getting display line segments.
        let lines = tmpl.display_line_segments(&[]);

        // Then the first line has trailing \ but the last doesn't.
        let first_line_text: String = lines[0].iter().map(|s| s.text.as_str()).collect();
        let last_line_text: String = lines[1].iter().map(|s| s.text.as_str()).collect();
        assert!(first_line_text.ends_with('\\'));
        assert!(!last_line_text.ends_with('\\'));
    }

    #[rstest::rstest]
    fn display_line_segments_substitutes_named_params() {
        // Given a command with named params and &&.
        let tmpl = CommandTemplate::parse("mkdir <branch> && cd <branch>");

        // When getting display line segments with an arg.
        let lines = tmpl.display_line_segments(&["my-feature".to_owned()]);

        // Then <branch> is replaced with "my-feature" and tagged with param index.
        // Line 1: static("mkdir "), param("my-feature", 0), static(" \\").
        assert_eq!(lines[0].len(), 3);
        assert_eq!(lines[0][0], DisplaySegment::static_text("mkdir "));
        assert_eq!(lines[0][1], DisplaySegment::param("my-feature", 0));
        assert_eq!(lines[0][2], DisplaySegment::static_text(" \\"));

        // Line 2: static("  && "), static("cd "), param("my-feature", 0).
        assert_eq!(lines[1][0], DisplaySegment::static_text("  && "));
        assert_eq!(lines[1][1], DisplaySegment::static_text("cd "));
        assert_eq!(lines[1][2], DisplaySegment::param("my-feature", 0));
    }

    #[rstest::rstest]
    fn display_line_segments_substitutes_positional_params() {
        // Given a command with $1 $2 and &&.
        let tmpl = CommandTemplate::parse("script.sh $1 && other.sh $2");

        // When getting display line segments with args.
        let lines = tmpl.display_line_segments(&["foo".to_owned(), "bar".to_owned()]);

        // Then params are replaced in display form (<1> and <2> substituted).
        // Line 1: static("script.sh "), param("foo", 0), static(" \\").
        assert_eq!(lines[0].len(), 3);
        assert_eq!(lines[0][0], DisplaySegment::static_text("script.sh "));
        assert_eq!(lines[0][1], DisplaySegment::param("foo", 0));

        // Line 2: static("  && "), static("other.sh "), param("bar", 1).
        assert_eq!(lines[1][1], DisplaySegment::static_text("other.sh "));
        assert_eq!(lines[1][2], DisplaySegment::param("bar", 1));
    }

    #[rstest::rstest]
    fn display_line_segments_unfilled_params_keep_placeholder() {
        // Given a command with two named params but only one arg provided.
        let tmpl = CommandTemplate::parse("mkdir <branch> && cd <target>");

        // When getting display line segments with only one arg.
        let lines = tmpl.display_line_segments(&["my-feature".to_owned()]);

        // Then the first param is substituted and the second keeps its placeholder.
        // Line 1: static("mkdir "), param("my-feature", 0).
        assert_eq!(lines[0][1], DisplaySegment::param("my-feature", 0));

        // Line 2: static("  && "), static("cd "), param("<target>", 1).
        assert_eq!(lines[1][1], DisplaySegment::static_text("cd "));
        assert_eq!(lines[1][2], DisplaySegment::param("<target>", 1));
    }

    #[rstest::rstest]
    fn display_line_segments_no_args_shows_all_placeholders() {
        // Given a command with named params.
        let tmpl = CommandTemplate::parse("mkdir <branch> && cd <branch>");

        // When getting display line segments with no args.
        let lines = tmpl.display_line_segments(&[]);

        // Then all <branch> placeholders are preserved and tagged with param index.
        assert_eq!(lines[0][1], DisplaySegment::param("<branch>", 0));
        assert_eq!(lines[1][2], DisplaySegment::param("<branch>", 0));
    }

    #[rstest::rstest]
    fn display_line_segments_mixed_named_and_positional() {
        // Given a command with mixed param types.
        let tmpl = CommandTemplate::parse("script.sh <branch> $1 && echo done");

        // When getting display line segments with both args.
        let lines = tmpl.display_line_segments(&["my-branch".to_owned(), "extra".to_owned()]);

        // Then both params are substituted.
        // Line 1: static("script.sh "), param("my-branch", 0), static(" "), param("extra", 1), ...
        assert_eq!(lines[0][1], DisplaySegment::param("my-branch", 0));
        assert_eq!(lines[0][3], DisplaySegment::param("extra", 1));
    }

    // --- Safe render: missing args ---

    #[rstest::rstest]
    fn render_missing_named_param_substitutes_empty() {
        // Given a template with two named params but only one arg.
        let tmpl = CommandTemplate::parse("script.sh <branch> <target>");

        // When rendering with missing args.
        let result = tmpl.render(&["my-branch".to_owned()]);

        // Then the missing param is replaced with empty string (no panic).
        assert_eq!(result, "script.sh my-branch ");
    }

    #[rstest::rstest]
    fn render_missing_positional_param_substitutes_empty() {
        // Given a template with $1 $2 but only one arg.
        let tmpl = CommandTemplate::parse("script.sh $1 $2");

        // When rendering with missing args.
        let result = tmpl.render(&["first".to_owned()]);

        // Then the missing param is replaced with empty string (no panic).
        assert_eq!(result, "script.sh first ");
    }

    #[rstest::rstest]
    fn render_empty_args_list_does_not_panic() {
        // Given a template with params but no args.
        let tmpl = CommandTemplate::parse("script.sh $1 $2");

        // When rendering with empty args list.
        let result = tmpl.render(&[]);

        // Then no panic — missing params replaced with empty.
        assert_eq!(result, "script.sh  ");
    }

    // --- Quoted arg parsing ---

    #[rstest::rstest]
    fn parse_quoted_args_empty_input() {
        assert_eq!(parse_quoted_args(""), Vec::<String>::new());
    }

    #[rstest::rstest]
    fn parse_quoted_args_whitespace_only() {
        assert_eq!(parse_quoted_args("   "), Vec::<String>::new());
    }

    #[rstest::rstest]
    fn parse_quoted_args_unquoted_single() {
        assert_eq!(parse_quoted_args("foo"), vec!["foo".to_owned()]);
    }

    #[rstest::rstest]
    fn parse_quoted_args_unquoted_multiple() {
        assert_eq!(
            parse_quoted_args("foo bar baz"),
            vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()]
        );
    }

    #[rstest::rstest]
    fn parse_quoted_args_quoted_single() {
        assert_eq!(parse_quoted_args("\"foo bar\""), vec!["foo bar".to_owned()]);
    }

    #[rstest::rstest]
    fn parse_quoted_args_quoted_preserves_internal_spaces() {
        assert_eq!(
            parse_quoted_args("\"hello   world\""),
            vec!["hello   world".to_owned()]
        );
    }

    #[rstest::rstest]
    fn parse_quoted_args_mixed_quoted_and_unquoted() {
        assert_eq!(
            parse_quoted_args("a \"b c\" d"),
            vec!["a".to_owned(), "b c".to_owned(), "d".to_owned()]
        );
    }

    #[rstest::rstest]
    fn parse_quoted_args_quoted_at_start() {
        assert_eq!(
            parse_quoted_args("\"foo bar\" baz"),
            vec!["foo bar".to_owned(), "baz".to_owned()]
        );
    }

    #[rstest::rstest]
    fn parse_quoted_args_quoted_at_end() {
        assert_eq!(
            parse_quoted_args("foo \"bar baz\""),
            vec!["foo".to_owned(), "bar baz".to_owned()]
        );
    }

    #[rstest::rstest]
    fn parse_quoted_args_adjacent_quoted_tokens() {
        assert_eq!(
            parse_quoted_args("\"foo\"\"bar\""),
            vec!["foobar".to_owned()]
        );
    }

    #[rstest::rstest]
    fn parse_quoted_args_empty_quotes() {
        assert_eq!(parse_quoted_args("\"\""), Vec::<String>::new());
    }

    #[rstest::rstest]
    fn parse_quoted_args_empty_quotes_between_tokens() {
        assert_eq!(
            parse_quoted_args("foo \"\" bar"),
            vec!["foo".to_owned(), "bar".to_owned()]
        );
    }

    #[rstest::rstest]
    fn parse_quoted_args_unterminated_quote() {
        // Unterminated quote captures the rest as the token content.
        assert_eq!(parse_quoted_args("\"foo bar"), vec!["foo bar".to_owned()]);
    }

    #[rstest::rstest]
    fn parse_quoted_args_unterminated_quote_with_spaces() {
        // Unterminated quote: everything after " is one token.
        assert_eq!(
            parse_quoted_args("\"foo bar baz"),
            vec!["foo bar baz".to_owned()]
        );
    }

    // --- Backslash escape tests ---

    #[rstest::rstest]
    fn parse_quoted_args_escaped_quote_outside_quotes() {
        // Input: foo\"bar → parser sees \" as escaped quote → foo"bar
        assert_eq!(parse_quoted_args("foo\\\"bar"), vec!["foo\"bar".to_owned()]);
    }

    #[rstest::rstest]
    fn parse_quoted_args_escaped_quote_inside_quotes() {
        assert_eq!(
            parse_quoted_args("\"foo\\\"bar\""),
            vec!["foo\"bar".to_owned()]
        );
    }

    #[rstest::rstest]
    fn parse_quoted_args_escaped_backslash_outside_quotes() {
        assert_eq!(parse_quoted_args("foo\\\\bar"), vec!["foo\\bar".to_owned()]);
    }

    #[rstest::rstest]
    fn parse_quoted_args_escaped_backslash_inside_quotes() {
        assert_eq!(
            parse_quoted_args("\"foo\\\\bar\""),
            vec!["foo\\bar".to_owned()]
        );
    }

    #[rstest::rstest]
    fn parse_quoted_args_escaped_other_char() {
        // \\n → n (we don't interpret escape sequences, just strip the backslash).
        assert_eq!(parse_quoted_args("foo\\nbar"), vec!["foonbar".to_owned()]);
    }

    #[rstest::rstest]
    fn parse_quoted_args_trailing_backslash_outside_quotes() {
        // Trailing backslash at end of input — treat as literal.
        assert_eq!(parse_quoted_args("foo\\"), vec!["foo\\".to_owned()]);
    }

    #[rstest::rstest]
    fn parse_quoted_args_trailing_backslash_inside_quotes() {
        // Trailing backslash at end of input inside quotes — treat as literal.
        assert_eq!(parse_quoted_args("\"foo\\"), vec!["foo\\".to_owned()]);
    }

    #[rstest::rstest]
    fn parse_quoted_args_escaped_space_outside_quotes() {
        assert_eq!(parse_quoted_args("foo\\ bar"), vec!["foo bar".to_owned()]);
    }

    #[rstest::rstest]
    fn parse_quoted_args_complex_mixed() {
        // Complex input mixing quotes, escapes, and unquoted tokens.
        assert_eq!(
            parse_quoted_args("branch \"my feature\" target\\ dir"),
            vec![
                "branch".to_owned(),
                "my feature".to_owned(),
                "target dir".to_owned(),
            ]
        );
    }

    #[rstest::rstest]
    fn parse_quoted_args_multiple_spaces_between_tokens() {
        assert_eq!(
            parse_quoted_args("foo    bar"),
            vec!["foo".to_owned(), "bar".to_owned()]
        );
    }

    #[rstest::rstest]
    fn parse_quoted_args_leading_and_trailing_whitespace() {
        assert_eq!(
            parse_quoted_args("  foo bar  "),
            vec!["foo".to_owned(), "bar".to_owned()]
        );
    }

    // --- shell_quote ---

    #[rstest::rstest]
    fn shell_quote_safe_value_passes_through() {
        assert_eq!(shell_quote("hello"), "hello");
    }

    #[rstest::rstest]
    fn shell_quote_value_with_spaces_is_wrapped() {
        assert_eq!(shell_quote("my branch"), "'my branch'");
    }

    #[rstest::rstest]
    fn shell_quote_empty_string_is_empty_quotes() {
        assert_eq!(shell_quote(""), "''");
    }

    #[rstest::rstest]
    fn shell_quote_embedded_single_quote_is_escaped() {
        assert_eq!(shell_quote("it's here"), "'it'\\''s here'");
    }

    #[rstest::rstest]
    fn shell_quote_dollar_sign_is_quoted() {
        assert_eq!(shell_quote("$HOME"), "'$HOME'");
    }

    #[rstest::rstest]
    fn shell_quote_semicolon_is_quoted() {
        assert_eq!(shell_quote("foo;bar"), "'foo;bar'");
    }

    #[rstest::rstest]
    fn shell_quote_pipe_is_quoted() {
        assert_eq!(shell_quote("foo|bar"), "'foo|bar'");
    }

    #[rstest::rstest]
    fn shell_quote_path_with_slash_is_safe() {
        // Forward slashes are not shell-special.
        assert_eq!(shell_quote("/tmp/workdir"), "/tmp/workdir");
    }

    #[rstest::rstest]
    fn shell_quote_hyphenated_value_is_safe() {
        assert_eq!(shell_quote("my-branch"), "my-branch");
    }

    // --- split_preserving_quotes ---

    #[rstest::rstest]
    fn split_preserving_quotes_keeps_quotes() {
        assert_eq!(
            split_preserving_quotes("\"my branch\" target"),
            vec!["\"my branch\"".to_owned(), "target".to_owned()]
        );
    }

    #[rstest::rstest]
    fn split_preserving_quotes_no_quotes() {
        assert_eq!(
            split_preserving_quotes("foo bar"),
            vec!["foo".to_owned(), "bar".to_owned()]
        );
    }

    #[rstest::rstest]
    fn split_preserving_quotes_empty_input() {
        assert_eq!(split_preserving_quotes(""), Vec::<String>::new());
    }

    #[rstest::rstest]
    fn split_preserving_quotes_single_quoted_arg() {
        assert_eq!(
            split_preserving_quotes("\"hello world\""),
            vec!["\"hello world\"".to_owned()]
        );
    }

    #[rstest::rstest]
    fn split_preserving_quotes_unterminated_quote() {
        assert_eq!(
            split_preserving_quotes("\"foo bar"),
            vec!["\"foo bar".to_owned()]
        );
    }
}
