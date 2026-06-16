Feature: App World Smoke Test
  Verify the AppWorld initializes correctly with the full actor system.

  Scenario: Fresh app world initializes correctly
    Given a fresh app
    Then the mode should be input
