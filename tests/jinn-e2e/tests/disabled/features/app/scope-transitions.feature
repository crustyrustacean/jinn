Feature: Scope Transitions
  Scope stack management through intents.

  Scenario: Input mode is the default
    Given a fresh app
    Then the mode should be input

  Scenario: Input mode can be entered
    Given a fresh app
    And the app is in input mode
    Then the mode should be input

  Scenario: Escape from input returns to normal
    Given a fresh app
    And the app is in input mode
    When the user presses esc
    Then the mode should be normal
