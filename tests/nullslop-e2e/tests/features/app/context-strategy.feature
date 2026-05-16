Feature: Context Strategy
  Prompt strategy switching and its effects on session state.

  Scenario: Switching to sliding_window updates session state
    Given a fresh app
    When the app submits a SwitchPromptStrategy with sliding_window
    Then the session strategy should be sliding_window

  Scenario: Switching to token_budget updates session state
    Given a fresh app
    When the app submits a SwitchPromptStrategy with token_budget
    Then the session strategy should be token_budget

  Scenario: Switching to compaction updates session state
    Given a fresh app
    When the app submits a SwitchPromptStrategy with compaction
    Then the session strategy should be compaction
