Feature: TUI Application
  End-to-end scenarios exercising keystroke → keymap → intent → state changes.

  Scenario: App starts in Normal mode
    Given a new app
    Then the which-key scope should be Normal
    And which-key should be inactive

  Scenario: Pressing 'i' in Normal mode enters Input mode
    Given a new app
    When the user presses i
    Then the mode should be Input

  Scenario: Pressing 'q' in Normal mode quits
    Given a new app
    When the user presses q
    Then the app should quit

  Scenario: Pressing Esc in Input mode returns to Normal
    Given a new app
    And the app is in Input mode
    When the user presses esc
    Then the mode should be Normal

  Scenario: Toggle which-key popup
    Given a new app
    When the app routes the ToggleWhichKey command
    Then which-key should be active

  Scenario: Shift+Enter inserts a newline in Input mode
    Given a new app
    And the app is in Input mode
    And the input buffer contains "hello"
    When the user presses enter with shift
    Then the input buffer should be "hello\n"
    And the chat history should contain 0 entry

  Scenario: Ctrl+Enter inserts a newline in Input mode
    Given a new app
    And the app is in Input mode
    And the input buffer contains "hello"
    When the user presses enter with ctrl
    Then the input buffer should be "hello\n"
    And the chat history should contain 0 entry

  Scenario: Cursor navigates back to first line after inserting a newline
    Given a new app
    And the app is in Input mode
    And the input buffer contains "hello"
    When the user presses enter with shift
    And the user presses w
    And the user presses up
    Then the cursor should be on row 0 col 1
    And the input buffer should be "hello\nw"
