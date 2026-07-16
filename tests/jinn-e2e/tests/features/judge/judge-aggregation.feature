Feature: Judge aggregation with majority vote
  Multiple judge instances evaluate the same origin turn in parallel.
  The last judge to finish tallies all verdicts and emits exactly one
  result: strict majority pass -> pass; otherwise (fail-majority or tie)
  -> fail with the failed reasons concatenated.

  Scenario: Single judge passes
    Given a fresh app
    And the active provider is set
    And the app attaches the plugin "judge"
    And the app queues a scripted origin turn with text "draft response"
    And the app queues a scripted judgment "judgment_passed" with message ""
    When the app submits an EnqueueUserMessage with text "hello"
    Then the origin session final entry is a transient "Judgment passed"

  Scenario: Single judge fails
    Given a fresh app
    And the active provider is set
    And the app attaches the plugin "judge"
    And the app queues a scripted origin turn with text "draft response"
    And the app queues a scripted judgment "judgment_failed" with message "too short"
    When the app submits an EnqueueUserMessage with text "hello"
    Then the origin session final entry is a failed user message containing "too short"

  Scenario: Majority 2 pass 1 fail emits pass
    Given a fresh app
    And the active provider is set
    And the app attaches the plugin "judge"
    And the app attaches the plugin "judge"
    And the app attaches the plugin "judge"
    And the app queues a scripted origin turn with text "draft response"
    And the app queues a scripted judgment "judgment_passed" with message ""
    And the app queues a scripted judgment "judgment_passed" with message ""
    And the app queues a scripted judgment "judgment_failed" with message "bad"
    When the app submits an EnqueueUserMessage with text "hello"
    Then the origin session final entry is a transient "Judgment passed"

  Scenario: Majority 1 pass 2 fail emits fail
    Given a fresh app
    And the active provider is set
    And the app attaches the plugin "judge"
    And the app attaches the plugin "judge"
    And the app attaches the plugin "judge"
    And the app queues a scripted origin turn with text "draft response"
    And the app queues a scripted judgment "judgment_passed" with message ""
    And the app queues a scripted judgment "judgment_failed" with message "off-topic"
    And the app queues a scripted judgment "judgment_failed" with message "too short"
    When the app submits an EnqueueUserMessage with text "hello"
    Then the origin session final entry is a failed user message containing "off-topic"
    And the origin session final entry is a failed user message containing "too short"

  Scenario: Tie 1 pass 1 fail counts as fail
    Given a fresh app
    And the active provider is set
    And the app attaches the plugin "judge"
    And the app attaches the plugin "judge"
    And the app queues a scripted origin turn with text "draft response"
    And the app queues a scripted judgment "judgment_passed" with message ""
    And the app queues a scripted judgment "judgment_failed" with message "inaccurate"
    When the app submits an EnqueueUserMessage with text "hello"
    Then the origin session final entry is a failed user message containing "inaccurate"

  Scenario: Fail is one-shot and disables the judge instance
    Given a fresh app
    And the active provider is set
    And the app attaches the plugin "judge"
    And the app queues a scripted origin turn with text "draft response"
    And the app queues a scripted judgment "judgment_failed" with message "too short"
    When the app submits an EnqueueUserMessage with text "hello"
    Then the origin session final entry is a failed user message containing "too short"
    And the judge plugin instance is disabled
