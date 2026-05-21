//! Retry decorator for [`LlmService`] with exponential backoff and jitter.

use std::time::Duration;

use error_stack::Report;

use crate::llm_message::LlmMessage;
use crate::service::{ChatStream, LlmService, LlmServiceError, ToolStream};
use crate::tool_types::ToolDefinition;

/// Callback invoked when a retry is about to happen.
///
/// Implementors can use this to surface retry status to the user.
pub trait OnRetry: Send + Sync {
    /// Called before sleeping before a retry attempt.
    ///
    /// - `attempt`: 1-indexed retry number (first retry = 1)
    /// - `max_retries`: maximum number of retries
    /// - `wait_duration`: how long we'll wait before retrying
    /// - `error`: the error that triggered the retry
    fn on_retry(
        &self,
        attempt: u32,
        max_retries: u32,
        wait_duration: Duration,
        error: &Report<LlmServiceError>,
    );
}

/// A no-op retry callback that does nothing.
pub struct NoOpOnRetry;

impl OnRetry for NoOpOnRetry {
    fn on_retry(&self, _: u32, _: u32, _: Duration, _: &Report<LlmServiceError>) {}
}

/// Configuration for retry behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    pub max_retries: u32,
    /// Base delay for exponential backoff.
    pub base_delay: Duration,
    /// Maximum delay cap for exponential backoff.
    /// Overridden by provider-supplied hints (Retry-After / error body).
    pub max_delay: Duration,
    /// Jitter amount as a fraction (0.0 to 1.0) of the computed delay.
    /// A value of 0.0 means no jitter; 1.0 means full jitter.
    pub jitter_fraction: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(60),
            jitter_fraction: 0.5,
        }
    }
}

/// Decorator that wraps an [`LlmService`] and retries on retryable errors.
///
/// Retries [`LlmServiceError::RateLimited`] and [`LlmServiceError::Retryable`]
/// errors with exponential backoff and jitter. Provider-supplied timing hints
/// (from the `RateLimited` variant) override the backoff calculation.
pub struct RetryingLlmService {
    inner: Box<dyn LlmService>,
    config: RetryConfig,
    on_retry: Box<dyn OnRetry>,
}

impl RetryingLlmService {
    /// Create a new retrying decorator.
    #[must_use]
    pub fn new(
        inner: Box<dyn LlmService>,
        config: RetryConfig,
        on_retry: Box<dyn OnRetry>,
    ) -> Self {
        Self {
            inner,
            config,
            on_retry,
        }
    }

    /// Compute the delay for the next retry attempt.
    ///
    /// Returns `None` if the error is not retryable.
    fn compute_delay(&self, attempt: u32, error: &Report<LlmServiceError>) -> Option<Duration> {
        let (is_retryable, provider_hint) = match error.downcast_ref::<LlmServiceError>() {
            Some(LlmServiceError::RateLimited { retry_after }) => (true, *retry_after),
            Some(LlmServiceError::Retryable) => (true, None),
            _ => (false, None),
        };

        if !is_retryable {
            return None;
        }

        // If provider gave us a hint, always use it (even if > max_delay).
        if let Some(hint) = provider_hint {
            return Some(hint);
        }

        // Exponential backoff: base_delay * 2^attempt
        let base_secs = self.config.base_delay.as_secs_f64();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let exponential = base_secs * 2_f64.powi(i32::try_from(attempt).unwrap_or(i32::MAX));
        let capped = exponential.min(self.config.max_delay.as_secs_f64());

        // Apply jitter: subtract a random fraction of jitter_fraction * capped
        let jitter_range = capped * self.config.jitter_fraction;
        let jitter = if jitter_range > 0.0 {
            rand::random_range(0.0..jitter_range)
        } else {
            0.0
        };
        let final_delay = (capped - jitter).max(0.0);

        Some(Duration::from_secs_f64(final_delay))
    }

    /// Run a retry loop for a fallible async operation.
    async fn retry_loop<F, Fut, T>(&self, f: F) -> Result<T, Report<LlmServiceError>>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T, Report<LlmServiceError>>>,
    {
        let mut attempt = 0u32;
        loop {
            match f().await {
                Ok(result) => return Ok(result),
                Err(report) => {
                    let delay = self.compute_delay(attempt, &report);
                    if attempt >= self.config.max_retries || delay.is_none() {
                        return Err(report);
                    }
                    let wait = delay.expect("checked above");
                    attempt += 1;
                    self.on_retry
                        .on_retry(attempt, self.config.max_retries, wait, &report);
                    tokio::time::sleep(wait).await;
                }
            }
        }
    }
}

impl std::fmt::Debug for RetryingLlmService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetryingLlmService")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl LlmService for RetryingLlmService {
    async fn chat_stream(
        &self,
        messages: Vec<LlmMessage>,
    ) -> Result<ChatStream, Report<LlmServiceError>> {
        self.retry_loop(|| self.inner.chat_stream(messages.clone()))
            .await
    }

    async fn chat_stream_with_tools(
        &self,
        messages: Vec<LlmMessage>,
        tools: Vec<ToolDefinition>,
    ) -> Result<ToolStream, Report<LlmServiceError>> {
        self.retry_loop(|| {
            self.inner
                .chat_stream_with_tools(messages.clone(), tools.clone())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use std::sync::Mutex;

    /// A service that fails N times then succeeds.
    struct FlakyService {
        fail_count: Mutex<u32>,
        fail_with: LlmServiceError,
    }

    impl FlakyService {
        fn new(fail_count: u32, fail_with: LlmServiceError) -> Self {
            Self {
                fail_count: Mutex::new(fail_count),
                fail_with,
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmService for FlakyService {
        async fn chat_stream(
            &self,
            _messages: Vec<LlmMessage>,
        ) -> Result<ChatStream, Report<LlmServiceError>> {
            let mut count = self.fail_count.lock().expect("lock");
            if *count > 0 {
                *count -= 1;
                return Err(Report::new(self.fail_with.clone()));
            }
            // Return an empty stream.
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    impl std::fmt::Debug for FlakyService {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("FlakyService").finish_non_exhaustive()
        }
    }

    /// Clone implementation for test error types.
    /// We need this because FlakyService::fail_with needs to be Clone-able.
    impl Clone for LlmServiceError {
        fn clone(&self) -> Self {
            match self {
                Self::ApiKey => Self::ApiKey,
                Self::Provider => Self::Provider,
                Self::Config => Self::Config,
                Self::RateLimited { retry_after } => Self::RateLimited {
                    retry_after: *retry_after,
                },
                Self::Retryable => Self::Retryable,
            }
        }
    }

    /// A recording callback that captures retry events.
    struct RecordingOnRetry {
        calls: std::sync::Arc<Mutex<Vec<(u32, u32, Duration)>>>,
    }

    impl RecordingOnRetry {
        fn new() -> Self {
            Self {
                calls: std::sync::Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl OnRetry for RecordingOnRetry {
        fn on_retry(
            &self,
            attempt: u32,
            max_retries: u32,
            wait: Duration,
            _error: &Report<LlmServiceError>,
        ) {
            self.calls
                .lock()
                .expect("lock")
                .push((attempt, max_retries, wait));
        }
    }

    #[tokio::test]
    async fn retry_succeeds_after_transient_failure() {
        // Given a service that fails once with Retryable.
        let inner = FlakyService::new(1, LlmServiceError::Retryable);
        let callback = RecordingOnRetry::new();
        let calls = callback.calls.clone();
        let svc = RetryingLlmService::new(
            Box::new(inner),
            RetryConfig {
                max_retries: 5,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(10),
                jitter_fraction: 0.0,
            },
            Box::new(callback),
        );

        // When calling chat_stream.
        let result = svc.chat_stream(vec![]).await;

        // Then it succeeds after one retry.
        assert!(result.is_ok());
        let calls = calls.lock().expect("lock");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, 1); // attempt 1
    }

    #[tokio::test]
    async fn retry_gives_up_after_max_retries() {
        // Given a service that always fails with Retryable.
        let inner = FlakyService::new(100, LlmServiceError::Retryable);
        let callback = RecordingOnRetry::new();
        let calls = callback.calls.clone();
        let svc = RetryingLlmService::new(
            Box::new(inner),
            RetryConfig {
                max_retries: 3,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(10),
                jitter_fraction: 0.0,
            },
            Box::new(callback),
        );

        // When calling chat_stream.
        let result = svc.chat_stream(vec![]).await;

        // Then it fails after max retries.
        assert!(result.is_err());
        let calls = calls.lock().expect("lock");
        assert_eq!(calls.len(), 3);
    }

    #[tokio::test]
    async fn retry_does_not_retry_non_retryable_error() {
        // Given a service that fails with Provider error.
        let inner = FlakyService::new(1, LlmServiceError::Provider);
        let callback = RecordingOnRetry::new();
        let calls = callback.calls.clone();
        let svc = RetryingLlmService::new(
            Box::new(inner),
            RetryConfig {
                max_retries: 5,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(10),
                jitter_fraction: 0.0,
            },
            Box::new(callback),
        );

        // When calling chat_stream.
        let result = svc.chat_stream(vec![]).await;

        // Then it fails immediately without retry.
        assert!(result.is_err());
        let calls = calls.lock().expect("lock");
        assert!(calls.is_empty());
    }

    #[tokio::test]
    async fn retry_uses_provider_hint_over_exponential() {
        // Given a service that fails with RateLimited and a 50ms hint.
        let inner = FlakyService::new(
            1,
            LlmServiceError::RateLimited {
                retry_after: Some(Duration::from_millis(50)),
            },
        );
        let callback = RecordingOnRetry::new();
        let calls = callback.calls.clone();
        let svc = RetryingLlmService::new(
            Box::new(inner),
            RetryConfig {
                max_retries: 5,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(10),
                jitter_fraction: 0.0,
            },
            Box::new(callback),
        );

        // When calling chat_stream.
        let result = svc.chat_stream(vec![]).await;

        // Then it succeeds.
        assert!(result.is_ok());
        let calls = calls.lock().expect("lock");
        assert_eq!(calls.len(), 1);
        // The wait duration should be the provider hint, not exponential backoff.
        assert_eq!(calls[0].2, Duration::from_millis(50));
    }

    #[tokio::test]
    async fn provider_hint_exceeds_max_delay() {
        // Given a service that fails with RateLimited and a hint exceeding max_delay.
        let inner = FlakyService::new(
            1,
            LlmServiceError::RateLimited {
                retry_after: Some(Duration::from_millis(200)),
            },
        );
        let callback = RecordingOnRetry::new();
        let calls = callback.calls.clone();
        let svc = RetryingLlmService::new(
            Box::new(inner),
            RetryConfig {
                max_retries: 5,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(50),
                jitter_fraction: 0.0,
            },
            Box::new(callback),
        );

        // When calling chat_stream.
        let result = svc.chat_stream(vec![]).await;

        // Then it succeeds and the wait is 200ms (provider hint overrides max_delay).
        assert!(result.is_ok());
        let calls = calls.lock().expect("lock");
        assert_eq!(calls[0].2, Duration::from_millis(200));
    }

    #[tokio::test]
    async fn no_retry_when_max_retries_is_zero() {
        // Given a service that fails with Retryable but max_retries is 0.
        let inner = FlakyService::new(1, LlmServiceError::Retryable);
        let callback = RecordingOnRetry::new();
        let calls = callback.calls.clone();
        let svc = RetryingLlmService::new(
            Box::new(inner),
            RetryConfig {
                max_retries: 0,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(10),
                jitter_fraction: 0.0,
            },
            Box::new(callback),
        );

        // When calling chat_stream.
        let result = svc.chat_stream(vec![]).await;

        // Then it fails immediately.
        assert!(result.is_err());
        let calls = calls.lock().expect("lock");
        assert!(calls.is_empty());
    }

    #[test]
    fn backoff_doubles_each_attempt() {
        let svc = RetryingLlmService::new(
            Box::new(FlakyService::new(0, LlmServiceError::Retryable)),
            RetryConfig {
                max_retries: 5,
                base_delay: Duration::from_secs(2),
                max_delay: Duration::from_secs(60),
                jitter_fraction: 0.0,
            },
            Box::new(NoOpOnRetry),
        );

        // Attempt 0: 2 * 2^0 = 2s
        let d0 = svc.compute_delay(0, &Report::new(LlmServiceError::Retryable));
        assert_eq!(d0, Some(Duration::from_secs(2)));

        // Attempt 1: 2 * 2^1 = 4s
        let d1 = svc.compute_delay(1, &Report::new(LlmServiceError::Retryable));
        assert_eq!(d1, Some(Duration::from_secs(4)));

        // Attempt 2: 2 * 2^2 = 8s
        let d2 = svc.compute_delay(2, &Report::new(LlmServiceError::Retryable));
        assert_eq!(d2, Some(Duration::from_secs(8)));

        // Attempt 5: 2 * 2^5 = 64, capped at 60s
        let d5 = svc.compute_delay(5, &Report::new(LlmServiceError::Retryable));
        assert_eq!(d5, Some(Duration::from_secs(60)));
    }

    #[test]
    fn compute_delay_returns_none_for_non_retryable() {
        let svc = RetryingLlmService::new(
            Box::new(FlakyService::new(0, LlmServiceError::Retryable)),
            RetryConfig::default(),
            Box::new(NoOpOnRetry),
        );

        let d = svc.compute_delay(0, &Report::new(LlmServiceError::Provider));
        assert!(d.is_none());

        let d = svc.compute_delay(0, &Report::new(LlmServiceError::ApiKey));
        assert!(d.is_none());

        let d = svc.compute_delay(0, &Report::new(LlmServiceError::Config));
        assert!(d.is_none());
    }

    #[test]
    fn compute_delay_uses_provider_hint() {
        let svc = RetryingLlmService::new(
            Box::new(FlakyService::new(0, LlmServiceError::Retryable)),
            RetryConfig {
                max_retries: 5,
                base_delay: Duration::from_secs(2),
                max_delay: Duration::from_secs(10),
                jitter_fraction: 0.0,
            },
            Box::new(NoOpOnRetry),
        );

        let d = svc.compute_delay(
            0,
            &Report::new(LlmServiceError::RateLimited {
                retry_after: Some(Duration::from_secs(100)),
            }),
        );
        // Provider hint overrides max_delay.
        assert_eq!(d, Some(Duration::from_secs(100)));
    }
}
