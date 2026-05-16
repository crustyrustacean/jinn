Feature: Context Pins
  Pinning and unpinning chat entries.

  Scenario: Pin entry sets pin on session
    Given a fresh app
    And the active session has a system entry with text "important"
    When the app submits a PinChatEntry for the last entry with position TOP
    Then the session has pinned entries

  Scenario: Unpin entry removes pin from session
    Given a fresh app
    And the active session has a pinned TOP entry with text "important"
    When the app submits an UnpinChatEntry for the last entry
    Then the session has no pinned entries
