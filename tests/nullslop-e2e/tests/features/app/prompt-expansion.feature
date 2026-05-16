Feature: Prompt Template Expansion
  Verify that #template tokens in user messages are expanded when sent to the LLM.

  Scenario: Submitting a message with a known template token expands it
    Given a fresh app
    And the active provider is set
    And a prompt template "greeting" with body "You are a helpful assistant."
    And the app is in input mode
    And the input buffer contains "#greeting"
    When the user presses enter
    Then the chat history should contain at least 1 entry
    And the last user entry has display "#greeting" and expanded "You are a helpful assistant."

  Scenario: Submitting plain text has identical display and expanded
    Given a fresh app
    And the active provider is set
    And the app is in input mode
    And the input buffer contains "hello world"
    When the user presses enter
    Then the chat history should contain at least 1 entry
    And the last user entry has display "hello world" and expanded "hello world"

  Scenario: Mixed text with template token expands only the token
    Given a fresh app
    And the active provider is set
    And a prompt template "plan" with body "Create a detailed plan for the following task."
    And the app is in input mode
    And the input buffer contains "please #plan for this feature"
    When the user presses enter
    Then the chat history should contain at least 1 entry
    And the last user entry has display "please #plan for this feature" and expanded "please Create a detailed plan for the following task. for this feature"

  Scenario: Unknown template token is left as literal text
    Given a fresh app
    And the active provider is set
    And the app is in input mode
    And the input buffer contains "hello #unknown"
    When the user presses enter
    Then the chat history should contain at least 1 entry
    And the last user entry has display "hello #unknown" and expanded "hello #unknown"
