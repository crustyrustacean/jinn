Feature: Token Counting
  Token ledger records input and output tokens.

  Scenario: Token ledger starts empty
    Given a fresh app
    Then the token ledger should have 0 records


