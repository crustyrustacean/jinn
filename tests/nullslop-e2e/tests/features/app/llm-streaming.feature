Feature: LLM Streaming
  End-to-end streaming behavior through the full actor system.

  Scenario: ToolsRegistered event caches tool definitions
    Given a fresh app
    When the app submits a ToolsRegistered event with tool "web_search"
    Then the tool definitions should contain "web_search"

  Scenario: Multiple tool registrations accumulate
    Given a fresh app
    When the app submits a ToolsRegistered event with tool "web_search"
    And the app submits a ToolsRegistered event with tool "file_read"
    Then the tool definitions should contain "web_search"
    And the tool definitions should contain "file_read"
