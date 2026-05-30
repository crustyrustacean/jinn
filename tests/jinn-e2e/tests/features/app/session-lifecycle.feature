Feature: Session Lifecycle
  Session state transitions and message processing.

  Scenario: Session starts idle
    Given a fresh app
    Then the session should be idle

  Scenario: Enqueue user message queues when streaming
    Given a fresh app
    And the active session is streaming
    When the app submits an EnqueueUserMessage with text "queued"
    Then the session queue should have 1 messages

  Scenario: Enqueue user message queues when sending
    Given a fresh app
    And the active session is sending
    When the app submits an EnqueueUserMessage with text "queued"
    Then the session queue should have 1 messages

  Scenario: Context size is not cached by default
    Given a fresh app
    Then the context size should not be cached

  Scenario: Token ledger starts empty
    Given a fresh app
    Then the token ledger should have 0 records

  Scenario: Default strategy is passthrough
    Given a fresh app
    Then the session strategy should be passthrough
