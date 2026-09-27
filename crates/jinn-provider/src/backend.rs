//! Backend discriminator for LLM providers.
//!
//! Maps to provider configuration strings.

use wherror::Error;

/// Error type for invalid backend strings.
#[derive(Debug, Error)]
#[error("invalid backend")]
pub struct BackendError;

/// Supported LLM backend providers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    /// OpenAI (also covers OpenAI-compatible providers).
    OpenAI,
    /// Anthropic (Claude models).
    Anthropic,
    /// Ollama (local inference).
    Ollama,
    /// DeepSeek.
    DeepSeek,
    /// xAI (Grok models).
    XAI,
    /// Phind.
    Phind,
    /// Google (Gemini models).
    Google,
    /// Groq.
    Groq,
    /// Azure OpenAI.
    AzureOpenAI,
    /// ElevenLabs.
    ElevenLabs,
    /// Cohere.
    Cohere,
    /// Mistral.
    Mistral,
    /// OpenRouter.
    OpenRouter,
    /// HuggingFace.
    HuggingFace,
    /// LM Studio (local inference).
    LmStudio,
    /// ZAI.
    ZAI,
}

impl std::str::FromStr for Backend {
    type Err = BackendError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(Self::OpenAI),
            "anthropic" => Ok(Self::Anthropic),
            "ollama" => Ok(Self::Ollama),
            "deepseek" => Ok(Self::DeepSeek),
            "xai" => Ok(Self::XAI),
            "phind" => Ok(Self::Phind),
            "google" => Ok(Self::Google),
            "groq" => Ok(Self::Groq),
            "azure-openai" => Ok(Self::AzureOpenAI),
            "elevenlabs" => Ok(Self::ElevenLabs),
            "cohere" => Ok(Self::Cohere),
            "mistral" => Ok(Self::Mistral),
            "openrouter" => Ok(Self::OpenRouter),
            "huggingface" => Ok(Self::HuggingFace),
            "lmstudio" => Ok(Self::LmStudio),
            "zai" => Ok(Self::ZAI),
            _ => Err(BackendError),
        }
    }
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenAI => write!(f, "openai"),
            Self::Anthropic => write!(f, "anthropic"),
            Self::Ollama => write!(f, "ollama"),
            Self::DeepSeek => write!(f, "deepseek"),
            Self::XAI => write!(f, "xai"),
            Self::Phind => write!(f, "phind"),
            Self::Google => write!(f, "google"),
            Self::Groq => write!(f, "groq"),
            Self::AzureOpenAI => write!(f, "azure-openai"),
            Self::ElevenLabs => write!(f, "elevenlabs"),
            Self::Cohere => write!(f, "cohere"),
            Self::Mistral => write!(f, "mistral"),
            Self::OpenRouter => write!(f, "openrouter"),
            Self::HuggingFace => write!(f, "huggingface"),
            Self::LmStudio => write!(f, "lmstudio"),
            Self::ZAI => write!(f, "zai"),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;

    #[rstest::rstest]
    #[case("openai", Backend::OpenAI)]
    #[case("anthropic", Backend::Anthropic)]
    #[case("ollama", Backend::Ollama)]
    #[case("deepseek", Backend::DeepSeek)]
    #[case("xai", Backend::XAI)]
    #[case("phind", Backend::Phind)]
    #[case("google", Backend::Google)]
    #[case("groq", Backend::Groq)]
    #[case("azure-openai", Backend::AzureOpenAI)]
    #[case("elevenlabs", Backend::ElevenLabs)]
    #[case("cohere", Backend::Cohere)]
    #[case("mistral", Backend::Mistral)]
    #[case("openrouter", Backend::OpenRouter)]
    #[case("huggingface", Backend::HuggingFace)]
    #[case("lmstudio", Backend::LmStudio)]
    #[case("zai", Backend::ZAI)]
    #[case("OPENAI", Backend::OpenAI)]
    #[case("OpenRouter", Backend::OpenRouter)]
    fn from_str_parses_known_backends(#[case] input: &str, #[case] expected: Backend) {
        // Given a known backend name.
        // When parsing the backend name.
        let parsed = input.parse::<Backend>();

        // Then it resolves to the expected backend.
        assert_eq!(parsed.unwrap(), expected);
    }

    #[rstest::rstest]
    fn from_str_rejects_unknown() {
        // Given a name that is not a known backend.
        // When parsing it.
        let parsed = "not-a-backend".parse::<Backend>();

        // Then parsing fails.
        assert!(parsed.is_err());
    }

    #[rstest::rstest]
    fn display_roundtrips() {
        // Given a backend.
        let backend = Backend::OpenRouter;

        // When displaying it and parsing the result back.
        let roundtripped = backend.to_string().parse::<Backend>();

        // Then the parsed backend is the original one.
        assert_eq!(roundtripped.unwrap(), backend);
    }
}
