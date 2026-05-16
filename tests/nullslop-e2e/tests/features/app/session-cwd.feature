Feature: Session CWD Persistence
  Session working directory persists across session reload.

  Scenario: Session CWD persists after reload
    Given a fresh app
    And the active provider is set
    And the app is in input mode
    And the input buffer contains "hello"
    When the user presses enter
    Then the session CWD should not be empty
    When the session is saved and reloaded
    Then the session CWD should be preserved

  Scenario: Session with non-existent CWD falls back to global CWD
    Given a fresh app
    And the active provider is set
    And the session CWD is set to a non-existent path
    And the app is in input mode
    And the input buffer contains "hello"
    When the user presses enter
    And the session is saved and reloaded
    Then a warning about the missing CWD should appear
    And the session CWD should fall back to the global CWD
