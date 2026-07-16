Feature: Welcome global plugin posts startup greeting
  The welcome plugin is a global plugin that hooks `on_app_started` and
  posts a system entry to the startup session. This is the simplest
  global-plugin shape: no LLM, no tools, no attach step.

  Scenario: App start posts a welcome system entry
    Given a fresh app
    Then the active session history contains a system entry containing "Welcome to jinn"
