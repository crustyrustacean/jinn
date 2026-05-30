Feature: Context Tools and Templates
  Tool definitions and prompt template management.

  Scenario: Prompt template store starts without unknown templates
    Given a fresh app
    Then the prompt template store should not contain "nonexistent"

  Scenario: Tools registered caches definitions
    Given a fresh app
    When the app submits a ToolsRegistered event with tool "bash"
    Then the tool definitions should contain "bash"
