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
        let chars: Vec<char> = command.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if chars[i] == '$' && i + 1 < chars.len() {
                let next = chars[i + 1];
                if next.is_ascii_digit() && next != '0' {
                    let n = next.to_string().parse::<usize>().expect("single digit");
                    let param = Param::Positional(n);
                    if !params.contains(&param) {
                        params.push(param);
                    }
                    i += 2;
                    continue;
                } else if next == '@' || next == '*' {
                    let splat = Param::Splat;
                    if !params.contains(&splat) {
                        params.push(splat);
                    }
                    i += 2;
                    continue;
                }
            }

            if chars[i] == '<' {
                // Scan for closing `>`.
                let start = i + 1;
                let mut end = start;
                while end < chars.len() && chars[end] != '>' {
                    end += 1;
                }
                if end > start && end < chars.len() {
                    let name: String = chars[start..end].iter().collect();
                    let param = Param::Named(name);
                    if !params.contains(&param) {
                        params.push(param);
                    }
                    i = end + 1;
                    continue;
                }
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
        self.params.iter().filter(|p| !matches!(p, Param::Splat)).count()
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
                        args[arg_idx].clone()
                    } else {
                        String::new()
                    };
                    result = result.replace(&search, &replacement);
                    arg_idx += 1;
                }
                Param::Positional(_n) => {
                    let search = format!("{param}");
                    let replacement = if arg_idx < args.len() {
                        args[arg_idx].clone()
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
            let joined = args[arg_idx..].join(" ");
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
}

impl fmt::Display for CommandTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display())
    }
}

#[cfg(test)]
mod tests {
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
            &[Param::Named("foo".to_owned()), Param::Named("bar".to_owned())]
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
        assert_eq!(tmpl.render(&["my-branch".to_owned()]), "script.sh my-branch");
    }

    #[rstest::rstest]
    fn render_multiple_params() {
        let tmpl = CommandTemplate::parse("script.sh $1 $2");
        assert_eq!(tmpl.render(&["foo".to_owned(), "bar".to_owned()]), "script.sh foo bar");
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
        assert_eq!(tmpl.render(&["my-feature".to_owned()]), "script.sh my-feature");
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
        assert_eq!(tmpl.render(&["hello".to_owned()]), "echo hello > output.txt");
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
}
