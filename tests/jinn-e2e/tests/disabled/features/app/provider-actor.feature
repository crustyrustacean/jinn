Feature: Provider Actor
  Provider switching and model management.

  Scenario: Session model can be set
    Given a fresh app
    And the active provider is set
    Then the mode should be input
