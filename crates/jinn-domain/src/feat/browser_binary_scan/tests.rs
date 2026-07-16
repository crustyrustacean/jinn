#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

use std::path::PathBuf;
use std::sync::Arc;

use kameo::prelude::Spawn;

use crate::common::actor_deps::ActorDeps;
use crate::common::bus::test_harness::{TestHarness, await_recorded};
use crate::common::services::test_services::TestServices;
use crate::feat::browser::BrowserBinary;
use crate::feat::provider_infra::ProvidersConfig;
use crate::init::env_init_actor::EnvironmentLoaded;

use super::binary_resolver::{BinaryFamily, BinaryLocator};
use super::{BrowserBinaryScanActor, BrowserBinaryScanActorDeps, BrowserBinaryVerified};

/// A fake filesystem that reports Chrome present, Chromium present, both, or neither.
#[derive(Default)]
struct FakeFs {
    chrome: Option<PathBuf>,
    chromium: Option<PathBuf>,
}

impl BinaryLocator for FakeFs {
    fn candidates(&self, family: BinaryFamily) -> Vec<PathBuf> {
        match family {
            BinaryFamily::Chrome => self.chrome.iter().cloned().collect(),
            BinaryFamily::Chromium => self.chromium.iter().cloned().collect(),
            BinaryFamily::Bundled => Vec::new(),
        }
    }
    fn exists(&self, _path: &std::path::Path) -> bool {
        // The fake's candidates list IS the set of existing paths.
        true
    }
}

async fn harness_with_locator(
    config: BrowserBinary,
    locator: Arc<dyn BinaryLocator + Send + Sync>,
) -> (TestHarness, kameo::actor::ActorRef<BrowserBinaryScanActor>) {
    let harness = TestHarness::new().await;
    let mut services = TestServices::builder().build();
    services.bus = harness.bus();
    let deps = ActorDeps { services };
    let actor = BrowserBinaryScanActor::spawn(BrowserBinaryScanActorDeps {
        deps,
        config,
        locator,
    });
    actor.wait_for_startup().await;
    (harness, actor)
}

#[tokio::test]
async fn environment_loaded_with_present_binary_emits_verified() {
    // Given an actor configured for Auto where Chrome exists.
    let locator = Arc::new(FakeFs {
        chrome: Some(PathBuf::from("/usr/bin/google-chrome")),
        chromium: None,
    }) as Arc<dyn BinaryLocator + Send + Sync>;
    let (harness, _actor) = harness_with_locator(BrowserBinary::Auto, locator).await;

    // Subscribe to the verified event BEFORE publishing, then publish.
    let recorder = harness.spawn_recorder::<BrowserBinaryVerified>().await;
    harness
        .publish(EnvironmentLoaded {
            config: ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            },
        })
        .await;

    // When waiting for the event.
    let messages = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;

    // Then a BrowserBinaryVerified event was emitted for the Chrome family.
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].family, BinaryFamily::Chrome);
    assert_eq!(
        messages[0].path.as_deref(),
        Some(std::path::Path::new("/usr/bin/google-chrome"))
    );
}

#[tokio::test]
async fn environment_loaded_with_no_binary_falls_back_to_bundled() {
    // Given an actor configured for Chrome where neither binary exists.
    let locator = Arc::new(FakeFs::default()) as Arc<dyn BinaryLocator + Send + Sync>;
    let (harness, _actor) = harness_with_locator(BrowserBinary::Chrome, locator).await;

    // Subscribe to the verified event (resolution always yields Verified now).
    let recorder = harness.spawn_recorder::<BrowserBinaryVerified>().await;
    harness
        .publish(EnvironmentLoaded {
            config: ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            },
        })
        .await;

    // When waiting for the event.
    let messages = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;

    // Then a Verified event was emitted for the Bundled family with a fallback note.
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].family, BinaryFamily::Bundled);
    assert!(messages[0].path.is_none());
    assert!(messages[0].fallback_note.is_some());
}

#[tokio::test]
async fn auto_falls_back_to_chromium_when_chrome_absent() {
    // Given an actor configured for Auto where only Chromium exists.
    let locator = Arc::new(FakeFs {
        chrome: None,
        chromium: Some(PathBuf::from("/usr/bin/chromium")),
    }) as Arc<dyn BinaryLocator + Send + Sync>;
    let (harness, _actor) = harness_with_locator(BrowserBinary::Auto, locator).await;

    // Subscribe then publish.
    let recorder = harness.spawn_recorder::<BrowserBinaryVerified>().await;
    harness
        .publish(EnvironmentLoaded {
            config: ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            },
        })
        .await;

    // When waiting for the event.
    let messages = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].family, BinaryFamily::Chromium);
}
