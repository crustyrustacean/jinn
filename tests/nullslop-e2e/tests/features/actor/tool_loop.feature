Feature: Multi-turn tool loop
  The LLM actor should keep looping when it produces tool calls.
  When tool results come back, it starts a new stream. This continues
  until the LLM responds without tool calls.

# Disabled: Phase 7 — the actor world doesn't include coordinator/projector actors.
# Events from the LLM actor (StreamToken, StreamCompleted) go through the channel
# but without a projector to write them to state, the chat history stays empty.
# Re-enable when the actor e2e world is updated to include coordinator/projector.
#
#  Scenario: LLM calls a tool then finishes with text
#    Given a fresh actor world with the tool loop fake
#    When I submit SendToLlmProvider with the tool loop trigger
#    Then the chat history should contain at least 1 entries
#    And the session should be idle
