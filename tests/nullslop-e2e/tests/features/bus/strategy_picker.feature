Feature: Strategy Picker
  Handler-level scenarios for opening, filtering, navigating, and confirming the context strategy picker.

  Scenario: Open strategy picker via OpenPicker command
    Given a fresh bus with all handlers
    When I submit OpenPicker with kind "ContextAssembly"
    Then the mode should be Picker
    And the active picker kind should be "ContextAssembly"

  Scenario: Insert char filters strategy entries
    Given a fresh bus with all handlers
    And the app is in Picker mode for strategy selection
    When I submit PickerInsertChar with "s"
    Then the context strategy picker filter should be "s"

  Scenario: Backspace removes from strategy filter
    Given a fresh bus with all handlers
    And the app is in Picker mode for strategy selection
    When I submit PickerInsertChar with "s"
    And I submit PickerInsertChar with "l"
    And I submit PickerBackspace
    Then the context strategy picker filter should be "s"

  Scenario: Confirm strategy switches and updates default
    Given a fresh bus with all handlers
    And the app is in Picker mode for strategy selection
    And the context strategy picker selection is 1
    When I submit PickerConfirm
    Then the mode should be Normal
    And the default strategy should be "sliding_window"
    And a SwitchPromptStrategy command should have been submitted

  Scenario: Confirm first entry selects passthrough
    Given a fresh bus with all handlers
    And the app is in Picker mode for strategy selection
    When I submit PickerConfirm
    Then the mode should be Normal
    And the default strategy should be "passthrough"
    And a SwitchPromptStrategy command should have been submitted

  Scenario: Provider picker still works via OpenPicker
    Given a fresh bus with all handlers
    And services with an ollama provider
    When I submit OpenPicker with kind "Provider"
    Then the mode should be Picker
    And the active picker kind should be "Provider"
