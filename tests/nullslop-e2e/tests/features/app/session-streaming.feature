Feature: Session Streaming
  How the session actor processes StreamToken and StreamCompleted events.

  Scenario: Stream completed with Finished stops streaming
    Given a fresh app
    And the active session is streaming
    When the app submits a StreamCompleted with Finished reason
    Then the session should be idle

  Scenario: Stream completed with Canceled pushes error entry
    Given a fresh app
    And the active session is streaming
    When the app submits a StreamCompleted with Canceled reason
    Then the session should be idle
    And the history should contain an error entry with text "Cancelled"

  Scenario: Stream completed with ToolUse transitions to sending
    Given a fresh app
    And the active session is streaming
    When the app submits a StreamCompleted with ToolUse reason
    Then the session should be sending

  Scenario: Streaming session is not idle
    Given a fresh app
    And the active session is streaming
    Then the session should be streaming

  Scenario: Sending session is not idle
    Given a fresh app
    And the active session is sending
    Then the session should be sending
