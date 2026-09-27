//! Startup guidance for a missing provider configuration.

use jinn_kernel::protocol::ChatEntry;

/// The guidance entry shown when no provider API keys are found.
///
/// Instructs the user to create a `.env` file and names the `providers.toml`
/// whose comments document the available environment variables. Built as a
/// transient entry so it is excluded from LLM context.
pub fn no_api_keys_msg() -> ChatEntry {
    let config_path = jinn_provider_config::config_path()
        .to_string_lossy()
        .into_owned();

    let content = format!(
        "\
**No API keys found**

\
Create a `.env` file in your working directory with your API keys.
\
See `{config_path}` for available environment variables."
    );

    ChatEntry::transient(content)
}

#[cfg(test)]
mod tests {
    use super::no_api_keys_msg;
    use jinn_kernel::protocol::ChatEntryKind;

    #[rstest::rstest]
    fn no_api_keys_msg_is_transient_entry() {
        // When creating the no-api-keys message.
        let entry = no_api_keys_msg();

        // Then it is a Transient entry.
        assert!(matches!(entry.kind, ChatEntryKind::Transient(_)));
    }

    #[rstest::rstest]
    fn no_api_keys_msg_contains_guidance() {
        // When creating the no-api-keys message.
        let entry = no_api_keys_msg();

        // Then it mentions guidance keywords.
        let text = entry.text();
        assert!(text.contains("No API keys found"), "should mention header");
        assert!(text.contains(".env"), "should mention .env");
        assert!(
            text.contains("providers.toml"),
            "should mention providers.toml"
        );
    }
}
