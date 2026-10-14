//! Bash resolution for lifecycle and tool shells.
//!
//! jinn's scripted surfaces — session lifecycle commands and the `bash`
//! tool — promise **bash semantics**. They must not run in the user's
//! login shell: `$SHELL` may be fish, dash, or zsh, shells that reject
//! bash syntax such as brace expansion and `[[`. Environment access
//! happens only here, at resolution time, in one place.

/// Resolves the shell that scripted commands run under.
///
/// - Windows: bare `bash` resolves to `C:\Windows\System32\bash.exe`,
///   the WSL launcher — it routes commands into a Linux VM, breaking
///   native tooling and the Windows kill path. Git Bash is preferred
///   when installed; otherwise the `SHELL` env var, then bare `bash`.
/// - Unix: always `bash`. PATH resolution applies, so a user-shadowing
///   bash (e.g. homebrew) wins over `/bin/bash` — intended.
///
/// Read once per call site at startup; the environment is a global
/// namespace and is not consulted again after resolution.
#[must_use]
pub fn bash_shell() -> String {
    #[cfg(target_os = "windows")]
    {
        let git_bash = "C:\\Program Files\\Git\\bin\\bash.exe";
        if std::path::Path::new(git_bash).exists() {
            git_bash.to_owned()
        } else {
            std::env::var("SHELL").unwrap_or_else(|_| "bash".to_owned())
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        "bash".to_owned()
    }
}

#[cfg(test)]
mod tests {
    #[rstest::rstest]
    #[test]
    fn resolution_yields_a_non_empty_shell_name() {
        // When resolving the shell.
        let shell = super::bash_shell();

        // Then it is non-empty: the spawn path would fail confusingly on "".
        assert!(!shell.is_empty());
    }
}
