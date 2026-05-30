Feature: Session Persistence
  Session data survives app restart.

  Scenario: Strategy survives restart via passthrough default
    Given a fresh app
    Then the session strategy should be passthrough
    When we restart the app
    Then the session strategy should be passthrough

  Scenario: Tool definitions cleared on restart
    Given a fresh app
    When the app submits a ToolsRegistered event with tool "web_search"
    Then the tool definitions should contain "web_search"
    When we restart the app
    Then the tool definitions should not contain "web_search"
