//! Command template parser — extracts positional parameters from shell commands.
//!
//! A [`CommandTemplate`] parses a command string like `script.sh $1 $2 $1` and
//! extracts the unique positional parameters in order of first appearance
//! (`[$1, $2]`). It can then:
//!
//! - **Render** the command with concrete arguments (substituting `$1` → first arg, etc.)
//! - **Display** the command with human-readable `<param>` tokens
//!
//! # Parameter syntax
//!
//! - `$1` through `$9` — individual positional parameters
//! - `$@` and `$*` — "all args" sentinel (means the command accepts variable args)
//!
//! Parameters are deduplicated by position. `$1 $2 $1` produces `[$1, $2]`
//! and substitutes `$1` in both positions when rendering.

use std::fmt;

/// A parsed shell command template with extracted positional parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandTemplate {
    /// The original command string.
    source: String,
    /// Unique positional parameters in order of first appearance.
    /// E.g., `$1 $2 $1` → `[1, 2]`.
    params: Vec<usize>,
    /// Whether `$@` or `$*` was found (accepts variable number of args).
    has_splat: bool,
}

impl CommandTemplate {
    /// Parse a command string and extract positional parameters.
    ///
    /// Scans for `$1`–`$9`, `$@`, and `$*` tokens. Returns a template
    /// that can render the command with substituted args.
    #[must_use]
    pub fn parse(command: &str) -> Self {
        let mut params = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut has_splat = false;

        // Scan for $N, $@, $* tokens.
        // We look for $ followed by a digit (1-9), @, or *.
        let chars: Vec<char> = command.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '$' && i + 1 < chars.len() {
                let next = chars[i + 1];
                if next.is_ascii_digit() && next != '0' {
                    let n = next.to_string().parse::<usize>().expect("single digit");
                    if seen.insert(n) {
                        params.push(n);
                    }
                    i += 2;
                    continue;
                } else if next == '@' || next == '*' {
                    has_splat = true;
                    i += 2;
                    continue;
                }
            }
            i += 1;
        }

        Self {
            source: command.to_owned(),
            params,
            has_splat,
        }
    }

    /// Whether this template requires any arguments.
    pub fn has_params(&self) -> bool {
        !self.params.is_empty() || self.has_splat
    }

    /// The number of required positional arguments.
    ///
    /// Returns the highest parameter index. E.g., `$1 $3` → 3.
    /// Returns 0 if `$@`/`$*` is present without numbered params.
    #[must_use]
    pub fn param_count(&self) -> usize {
        if self.params.is_empty() && self.has_splat {
            return 0;
        }
        self.params.iter().max().copied().unwrap_or(0)
    }

    /// Render the command with concrete arguments substituted.
    ///
    /// Args are 0-indexed: `args[0]` → `$1`, `args[1]` → `$2`, etc.
    /// `$@` and `$*` are replaced with all args joined by spaces.
    ///
    /// # Panics
    ///
    /// Panics if the number of args doesn't match the required parameter count.
    pub fn render(&self, args: &[String]) -> String {
        let mut result = self.source.clone();

        // Replace numbered params in reverse order to avoid $1 conflicting with $10+.
        for &n in self.params.iter().rev() {
            assert!(
                n <= args.len(),
                "expected at least {n} args, got {}",
                args.len()
            );
            let arg = &args[n - 1];
            result = result.replace(&format!("${n}"), arg);
        }

        // Replace $@ and $* with all args joined.
        if self.has_splat {
            let joined = args.join(" ");
            result = result.replace("$@", &joined);
            result = result.replace("$*", &joined);
        }

        result
    }

    /// Render the command with `<param>` display tokens.
    ///
    /// `$1` → `<1>`, `$2` → `<2>`, `$@`/`$*` → `<args>`.
    /// Used for the arg-input popup UI.
    #[must_use]
    pub fn display(&self) -> String {
        let mut result = self.source.clone();

        // Replace numbered params in reverse order.
        for &n in self.params.iter().rev() {
            result = result.replace(&format!("${n}"), &format!("<{n}>"));
        }

        if self.has_splat {
            result = result.replace("$@", "<args>");
            result = result.replace("$*", "<args>");
        }

        result
    }

    /// The unique positional parameter indices in order of first appearance.
    #[must_use]
    pub fn params(&self) -> &[usize] {
        &self.params
    }

    /// Whether `$@` or `$*` was found in the command.
    #[must_use]
    pub fn has_splat(&self) -> bool {
        self.has_splat
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

    // --- Parsing ---

    #[rstest::rstest]
    fn parse_no_params() {
        // Given a command with no parameters.
        let tmpl = CommandTemplate::parse("echo hello");

        // Then it has no params.
        assert!(!tmpl.has_params());
        assert!(tmpl.params().is_empty());
        assert_eq!(tmpl.param_count(), 0);
    }

    #[rstest::rstest]
    fn parse_one_param() {
        // Given a command with $1.
        let tmpl = CommandTemplate::parse("script.sh $1");

        // Then it has one param.
        assert!(tmpl.has_params());
        assert_eq!(tmpl.params(), &[1]);
        assert_eq!(tmpl.param_count(), 1);
    }

    #[rstest::rstest]
    fn parse_multiple_params() {
        // Given a command with $1 and $2.
        let tmpl = CommandTemplate::parse("script.sh $1 $2");

        // Then it has two params in order.
        assert_eq!(tmpl.params(), &[1, 2]);
        assert_eq!(tmpl.param_count(), 2);
    }

    #[rstest::rstest]
    fn parse_deduplicates_repeated_params() {
        // Given a command with $1 appearing twice.
        let tmpl = CommandTemplate::parse("script.sh $1 $2 $1");

        // Then $1 appears once in params.
        assert_eq!(tmpl.params(), &[1, 2]);
        assert_eq!(tmpl.param_count(), 2);
    }

    #[rstest::rstest]
    fn parse_splat_at() {
        // Given a command with $@.
        let tmpl = CommandTemplate::parse("script.sh $@");

        // Then has_splat is true.
        assert!(tmpl.has_splat());
        assert!(tmpl.has_params());
    }

    #[rstest::rstest]
    fn parse_splat_star() {
        // Given a command with $*.
        let tmpl = CommandTemplate::parse("script.sh $*");

        // Then has_splat is true.
        assert!(tmpl.has_splat());
    }

    #[rstest::rstest]
    fn parse_mixed_numbered_and_splat() {
        // Given a command with both $1 and $@.
        let tmpl = CommandTemplate::parse("script.sh $1 $@");

        // Then both are detected.
        assert_eq!(tmpl.params(), &[1]);
        assert!(tmpl.has_splat());
    }

    #[rstest::rstest]
    fn parse_skips_dollar_zero() {
        // Given a command with $0 (shell script name, not a user param).
        let tmpl = CommandTemplate::parse("echo $0");

        // Then no params are extracted.
        assert!(!tmpl.has_params());
    }

    #[rstest::rstest]
    fn parse_non_consecutive_params() {
        // Given a command with $1 and $3 (no $2).
        let tmpl = CommandTemplate::parse("script.sh $1 $3");

        // Then both params are detected.
        assert_eq!(tmpl.params(), &[1, 3]);
        assert_eq!(tmpl.param_count(), 3);
    }

    // --- Rendering ---

    #[rstest::rstest]
    fn render_no_params() {
        // Given a template with no params.
        let tmpl = CommandTemplate::parse("echo hello");

        // When rendering with no args.
        let result = tmpl.render(&[]);

        // Then the command is unchanged.
        assert_eq!(result, "echo hello");
    }

    #[rstest::rstest]
    fn render_one_param() {
        // Given a template with $1.
        let tmpl = CommandTemplate::parse("script.sh $1");

        // When rendering with one arg.
        let result = tmpl.render(&["my-branch".to_owned()]);

        // Then $1 is replaced.
        assert_eq!(result, "script.sh my-branch");
    }

    #[rstest::rstest]
    fn render_multiple_params() {
        // Given a template with $1 and $2.
        let tmpl = CommandTemplate::parse("script.sh $1 $2");

        // When rendering with two args.
        let result = tmpl.render(&["foo".to_owned(), "bar".to_owned()]);

        // Then both are replaced.
        assert_eq!(result, "script.sh foo bar");
    }

    #[rstest::rstest]
    fn render_repeated_param() {
        // Given a template with $1 appearing twice.
        let tmpl = CommandTemplate::parse("script.sh $1 $2 $1");

        // When rendering.
        let result = tmpl.render(&["branch".to_owned(), "dir".to_owned()]);

        // Then both $1 occurrences are replaced.
        assert_eq!(result, "script.sh branch dir branch");
    }

    #[rstest::rstest]
    fn render_splat() {
        // Given a template with $@.
        let tmpl = CommandTemplate::parse("script.sh $@");

        // When rendering with multiple args.
        let result = tmpl.render(&["a".to_owned(), "b".to_owned(), "c".to_owned()]);

        // Then $@ is replaced with all args joined.
        assert_eq!(result, "script.sh a b c");
    }

    // --- Display ---

    #[rstest::rstest]
    fn display_no_params() {
        // Given a template with no params.
        let tmpl = CommandTemplate::parse("echo hello");

        // When displaying.
        let result = tmpl.display();

        // Then the command is unchanged.
        assert_eq!(result, "echo hello");
    }

    #[rstest::rstest]
    fn display_with_params() {
        // Given a template with $1 and $2.
        let tmpl = CommandTemplate::parse("script.sh $1 $2");

        // When displaying.
        let result = tmpl.display();

        // Then params are shown as <N> tokens.
        assert_eq!(result, "script.sh <1> <2>");
    }

    #[rstest::rstest]
    fn display_with_splat() {
        // Given a template with $@.
        let tmpl = CommandTemplate::parse("script.sh $@");

        // When displaying.
        let result = tmpl.display();

        // Then $@ is shown as <args>.
        assert_eq!(result, "script.sh <args>");
    }

    #[rstest::rstest]
    fn display_repeated_params() {
        // Given a template with $1 appearing twice.
        let tmpl = CommandTemplate::parse("script.sh $1 $2 $1");

        // When displaying.
        let result = tmpl.display();

        // Then all occurrences are replaced.
        assert_eq!(result, "script.sh <1> <2> <1>");
    }

    // --- Shell redirection safety ---

    #[rstest::rstest]
    fn display_does_not_confuse_redirection_with_params() {
        // Given a command with shell redirection.
        let tmpl = CommandTemplate::parse("echo $1 > output.txt");

        // When displaying.
        let result = tmpl.display();

        // Then only $1 is replaced, redirection is preserved.
        assert_eq!(result, "echo <1> > output.txt");
    }

    #[rstest::rstest]
    fn render_preserves_redirection() {
        // Given a command with redirection.
        let tmpl = CommandTemplate::parse("echo $1 > output.txt");

        // When rendering.
        let result = tmpl.render(&["hello".to_owned()]);

        // Then redirection is preserved.
        assert_eq!(result, "echo hello > output.txt");
    }

    // --- Display trait ---

    #[rstest::rstest]
    fn display_trait_delegates_to_display_method() {
        // Given a template with params.
        let tmpl = CommandTemplate::parse("script.sh $1 $2");

        // When using Display trait.
        let result = format!("{tmpl}");

        // Then it matches display().
        assert_eq!(result, tmpl.display());
    }
}
