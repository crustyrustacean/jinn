Feature: Session Fork
  End-to-end scenarios exercising the session fork picker workflow.

  Scenario: Opening fork picker shows user and assistant entries
    Given a new app
    And the active session has 2 user messages and 1 assistant messages
    When the user opens the fork picker
    Then the fork picker should be active
    And the fork picker should show 3 entries

  Scenario: Fork picker excludes non-user/assistant entries
    Given a new app
    And the active session has an actor message from "bash" with text "system output"
    And the active session has 1 user messages and 1 assistant messages
    When the user opens the fork picker
    Then the fork picker should show 2 entries

  Scenario: Confirming fork entry dispatches fork command
    Given a new app
    And the active session has 2 user messages and 1 assistant messages
    When the user opens the fork picker
    And the user confirms the fork picker selection
    Then the session should have changed

  Scenario: Fork picker entry count matches user plus assistant
    Given a new app
    And the active session has 3 user messages and 3 assistant messages
    When the user opens the fork picker
    Then the fork picker should show 6 entries

  Scenario: Empty session shows no fork entries
    Given a new app
    When the user opens the fork picker
    Then the fork picker should be active
    And the fork picker should show 0 entries
