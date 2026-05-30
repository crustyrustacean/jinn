Feature: No API Keys Guidance
  When no API keys are configured, a guidance message should appear.

  Scenario: No API keys guidance message appears on startup
    Given a fresh app
    Then the chat history should contain at least 2 entries
    And the last chat entry should contain "No API keys found"
