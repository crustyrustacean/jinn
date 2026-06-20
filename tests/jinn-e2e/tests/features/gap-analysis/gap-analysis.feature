Feature: Gap-analysis auto-fire on task list completion
  The gap-analysis plugin folds task-list completion state from
  `on_task_list_updated` and, when the list is complete and the session
  goes Idle, enqueues the expanded `#gap-analysis` body exactly once.

  Scenario: List completes on Idle fires once
    Given a fresh app
    And the active provider is set
    And the app has a prompt template "gap-analysis" with body "Run the gap analysis."
    And the app attaches the plugin "gap-analysis"
    When the app completes the task list then ends the turn
    Then the origin session history gains an expanded "gap-analysis" entry

  Scenario: Enqueued text is the expanded body, not the literal token
    Given a fresh app
    And the active provider is set
    And the app has a prompt template "gap-analysis" with body "Run the gap analysis."
    And the app attaches the plugin "gap-analysis"
    When the app completes the task list then ends the turn
    Then the origin session history has no user entry containing the literal "#gap-analysis" token

  Scenario: Pending task list does not enqueue
    Given a fresh app
    And the active provider is set
    And the app has a prompt template "gap-analysis" with body "Run the gap analysis."
    And the app attaches the plugin "gap-analysis"
    When the app sets a pending task list then ends the turn
    Then the origin session history has no expanded "gap-analysis" entry

  Scenario: No re-fire on the same completed list
    Given a fresh app
    And the active provider is set
    And the app has a prompt template "gap-analysis" with body "Run the gap analysis."
    And the app attaches the plugin "gap-analysis"
    When the app completes the task list then ends the turn
    And the app ends another turn without changing the list
    Then the origin session history has exactly one expanded "gap-analysis" entry

  Scenario: Re-fires after a new plan
    Given a fresh app
    And the active provider is set
    And the app has a prompt template "gap-analysis" with body "Run the gap analysis."
    And the app attaches the plugin "gap-analysis"
    When the app completes the task list then ends the turn
    And the app sets a pending task list then ends the turn
    And the app completes the task list then ends the turn
    Then the origin session history has exactly two expanded "gap-analysis" entries
