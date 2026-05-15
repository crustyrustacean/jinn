Feature: Chat Scroll
  Cursor and scroll behavior for the chat log.

  Scenario: First entry auto-selects cursor
    Given a fresh app
    And the active provider is set
    When the user presses i
    And the user presses h
    And the user presses enter
    Then the cursor should be on the last entry

  Scenario: Cursor advances across two turns
    Given a fresh app
    And the active provider is set
    When the user presses i
    And the user presses 1
    And the user presses enter
    Then the cursor should be on the last entry
    When the user presses i
    And the user presses 2
    And the user presses enter
    Then the cursor should be on the last entry

  Scenario: ScrollToBottom re-enables auto-scroll
    Given a fresh app
    And the active provider is set
    When the user presses i
    And the user presses 1
    And the user presses enter
    Then the cursor should be on the last entry
    When the user presses G
    And the user presses i
    And the user presses 2
    And the user presses enter
    Then the cursor should be on the last entry
