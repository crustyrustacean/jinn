Feature: Headless App Dispatch
  Running headless scripts through the headless app.

  Scenario: Headless script with quit key completes successfully
    Given a headless app
    When a script containing "q" is run
    Then the script should complete successfully

  Scenario: Headless script with missing file returns error
    Given a headless app
    When a script is run from a missing file
    Then the script should fail
