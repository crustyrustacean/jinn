#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

use jinn_testutil::{buffer_row, setup_term};

use crate::common::app_state::AppState;
use crate::common::render_ctx::RenderCtx;
use crate::common::ui_element::UiElement;
use crate::feat::session::model_selection::{AlloyStrategy, ModelSelection};
use crate::feat::session::token_stats::TokenRecord;
use crate::feat::ui::status_bar::element::StatusBarElement;

#[rstest::rstest]
fn name_returns_status_bar() {
    let element = StatusBarElement;
    assert_eq!(element.name(), "status-bar");
}

#[rstest::rstest]
fn render_shows_no_model_selected_when_unset() {
    let mut element = StatusBarElement;
    let state = AppState::default();
    let (mut terminal, area) = setup_term(50, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 50);
    assert!(row.starts_with("\u{2191}"));
    assert!(row.contains("no model selected"));
}

#[rstest::rstest]
fn render_shows_provider_and_model() {
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    let (mut terminal, area) = setup_term(50, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 50);
    assert!(row.starts_with("\u{2191}"));
    assert!(row.contains("(ollama)/llama3"));
}

#[rstest::rstest]
fn render_single_model_ignores_stale_ledger_model_used() {
    // Given a Single selection of ollama/llama3, but a token ledger whose last
    // record claims a different (stale) model was dispatched.
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    state.active_session_mut().push_token_record(TokenRecord {
        model_used: Some("openrouter/gpt-4".to_owned()),
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 1000,
        tokens_received: 500,
        cost: None,
    });

    // When rendering.
    let (mut terminal, area) = setup_term(50, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 50);

    // Then the selected model is shown, not the stale dispatched model.
    assert!(
        row.contains("(ollama)/llama3"),
        "should show selected model, got: {row}"
    );
    assert!(
        !row.contains("gpt-4"),
        "should not show stale dispatched model, got: {row}"
    );
}

#[rstest::rstest]
fn render_right_aligns_text() {
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    let (mut terminal, area) = setup_term(50, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    // Row 1 is the info line.
    let first = buffer.cell((0, 1)).expect("first cell");
    assert_eq!(first.symbol(), "\u{2191}");
    let last = buffer.cell((49, 1)).expect("last cell");
    assert_eq!(last.symbol(), "3");
}

#[rstest::rstest]
fn render_shows_provider_with_slash_in_model() {
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state.active_session_mut().set_model(ModelSelection::Single(
        "openrouter/anthropic/claude-sonnet-4".to_owned(),
    ));
    let (mut terminal, area) = setup_term(80, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 80);
    assert!(row.starts_with("\u{2191}"));
    assert!(row.contains("(openrouter)/anthropic/claude-sonnet-4"));
}

// --- Token display tests ---

#[rstest::rstest]
fn render_shows_token_counts_with_zero_values() {
    // Given a state with no token records.
    let mut element = StatusBarElement;
    let state = AppState::default();
    let (mut terminal, area) = setup_term(80, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 80);
    // Then the status bar shows zero token counts.
    assert!(row.contains("\u{2191}0 \u{2193}0"));
    // And cost is always shown as $0.00000.
    assert!(row.contains("$0.00000"));
}

#[rstest::rstest]
fn render_shows_token_counts_with_values() {
    // Given a session with token records.
    use crate::feat::session::token_stats::TokenRecord;
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    state.active_session_mut().push_token_record(TokenRecord {
        model_used: None,
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 1500,
        tokens_received: 750,
        cost: None,
    });
    let (mut terminal, area) = setup_term(80, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 80);
    // Then the status bar shows token counts.
    assert!(row.contains("1.5k"));
    assert!(row.contains("750"));
    // And cost is shown as $0.00000 when no cost data.
    assert!(row.contains("$0.00000"));
}

#[rstest::rstest]
fn render_shows_zero_percent_max_when_context_size_but_no_limit() {
    // Given a session with a cached context size but no model cache.
    use crate::feat::session::token_stats::TokenRecord;
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    state.active_session_mut().push_token_record(TokenRecord {
        model_used: None,
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 5000,
        tokens_received: 0,
        cost: None,
    });
    state.active_session_mut().set_context_size(5000);
    let (mut terminal, area) = setup_term(80, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 80);
    // Then the status bar shows usage with unknown limit (no context_length available).
    assert!(row.contains("5.0k/???"), "expected 5.0k/???, got: {row}");
}

#[rstest::rstest]
fn render_shows_zero_percent_max_when_no_context_size() {
    // Given a session with no cached context size.
    let mut element = StatusBarElement;
    let state = AppState::default();
    let (mut terminal, area) = setup_term(80, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 80);
    // Then the context display shows 0/??? as fallback (no context_size, no model cache).
    assert!(row.contains("0/???"), "expected 0/??? fallback, got: {row}");
    // And token counts are still shown.
    assert!(row.contains("0 0") || row.contains("\u{2191}0 \u{2193}0"));
}

// --- Turn counter tests ---

#[rstest::rstest]
fn render_shows_zero_turns_when_no_history() {
    // Given a state with no chat entries.
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    let (mut terminal, area) = setup_term(80, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 80);
    // Then the status bar shows "Turns: 0".
    assert!(row.contains("\u{21BB}0"));
}

#[rstest::rstest]
fn render_shows_turn_count_with_history() {
    // Given a state with user and assistant entries.
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    state
        .active_session_mut()
        .push_entry(crate::protocol::ChatEntry::user("hello"));
    state
        .active_session_mut()
        .push_entry(crate::protocol::ChatEntry::assistant("hi there"));
    state
        .active_session_mut()
        .push_entry(crate::protocol::ChatEntry::user("how are you?"));
    state
        .active_session_mut()
        .push_entry(crate::protocol::ChatEntry::assistant("doing well"));
    let (mut terminal, area) = setup_term(80, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 80);
    // Then the status bar shows "Turns: 4".
    assert!(row.contains("\u{21BB}4"));
}

#[rstest::rstest]
fn render_turn_count_skips_tool_loop_intermediates() {
    // Given a state with a tool-loop conversation.
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    state
        .active_session_mut()
        .push_entry(crate::protocol::ChatEntry::user("fix the bug"));
    state
        .active_session_mut()
        .push_entry(crate::protocol::ChatEntry::assistant("let me check"));
    state
        .active_session_mut()
        .push_entry(crate::protocol::ChatEntry::tool_call(
            "id-1",
            "bash",
            r#"{"command":"ls"}"#,
        ));
    state
        .active_session_mut()
        .push_entry(crate::protocol::ChatEntry::assistant("fixed it"));
    let (mut terminal, area) = setup_term(80, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 80);
    // Then only the user and final assistant count as turns.
    assert!(row.contains("\u{21BB}2"));
}

// --- CWD display tests ---

#[rstest::rstest]
fn render_shows_cwd_on_first_line() {
    // Given default state (cwd is "." which resolves to the current dir).
    let mut element = StatusBarElement;
    let state = AppState::default();
    let (mut terminal, area) = setup_term(80, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    // The first row should have content (cwd).
    let row0 = buffer_row(&buffer, 0, 80);
    assert!(!row0.trim().is_empty(), "cwd line should not be empty");
}

#[rstest::rstest]
fn render_shows_absolute_path_for_non_home_cwd() {
    // Given a session with a non-home CWD.
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    state
        .active_session_mut()
        .set_cwd(std::path::PathBuf::from("/tmp/test-project"));
    let (mut terminal, area) = setup_term(80, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    // The cwd line (row 0) should show the full absolute path.
    let row0 = buffer_row(&buffer, 0, 80);
    assert!(
        row0.contains("/tmp/test-project"),
        "expected /tmp/test-project in cwd line, got: {row0}"
    );
}

#[rstest::rstest]
fn render_shows_tilde_for_home_cwd() {
    // Given a session whose CWD is the user's home directory.
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    let home = dirs::home_dir().expect("home dir exists");
    state.active_session_mut().set_cwd(home);
    let (mut terminal, area) = setup_term(80, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    // The cwd line should show just "~" (home dir).
    let row0 = buffer_row(&buffer, 0, 80);
    assert!(
        row0.trim_start().starts_with('~'),
        "expected ~ in cwd line, got: {row0}"
    );
}

#[rstest::rstest]
fn render_shows_tilde_substitution_for_path_under_home() {
    // Given a session whose CWD is under the user's home directory.
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    let home = dirs::home_dir().expect("home dir exists");
    state
        .active_session_mut()
        .set_cwd(home.join("projects/my-app"));
    let (mut terminal, area) = setup_term(80, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    // The cwd line should show "~/projects/my-app".
    let row0 = buffer_row(&buffer, 0, 80);
    assert!(
        row0.trim_start().starts_with("~/projects/my-app"),
        "expected ~/projects/my-app in cwd line, got: {row0}"
    );
}

// --- Context limit display tests ---

#[rstest::rstest]
fn render_shows_context_limit_with_usage_and_percentage() {
    // Given a session with a cached context size and a model cache with context_length.
    use crate::feat::session::token_stats::TokenRecord;
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state.active_session_mut().set_model(ModelSelection::Single(
        "openrouter/anthropic/claude-sonnet-4".to_owned(),
    ));
    state.active_session_mut().push_token_record(TokenRecord {
        model_used: None,
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 5000,
        tokens_received: 0,
        cost: None,
    });
    state.active_session_mut().set_context_size(5000);

    // And a model cache with context_length for the active model.
    let mut cache_entries = std::collections::HashMap::new();
    cache_entries.insert(
        "openrouter".to_owned(),
        vec![crate::feat::provider_infra::ModelInfo {
            id: "anthropic/claude-sonnet-4".to_owned(),
            context_length: Some(200_000),
        }],
    );
    state.provider.model_cache = Some(crate::feat::provider_infra::ModelCache {
        entries: cache_entries,
        last_updated_at: None,
    });

    let (mut terminal, area) = setup_term(100, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 100);
    // Then the status bar shows the percentage and formatted max.
    assert!(row.contains("2.5%/200k"), "expected 2.5%/200k, got: {row}");
}

#[rstest::rstest]
fn render_falls_back_when_no_context_limit_in_cache() {
    // Given a session with a cached context size but no context_length in the model cache.
    use crate::feat::session::token_stats::TokenRecord;
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    state.active_session_mut().push_token_record(TokenRecord {
        model_used: None,
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 5000,
        tokens_received: 0,
        cost: None,
    });
    state.active_session_mut().set_context_size(5000);

    // Model cache exists but has no context_length.
    let mut cache_entries = std::collections::HashMap::new();
    cache_entries.insert(
        "ollama".to_owned(),
        vec![crate::feat::provider_infra::ModelInfo {
            id: "llama3".to_owned(),
            context_length: None,
        }],
    );
    state.provider.model_cache = Some(crate::feat::provider_infra::ModelCache {
        entries: cache_entries,
        last_updated_at: None,
    });

    let (mut terminal, area) = setup_term(80, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 80);
    // Then the status bar shows usage with unknown limit (model found but no context_length).
    assert!(
        row.contains("5.0k/???"),
        "expected 5.0k/??? fallback, got: {row}"
    );
}

#[rstest::rstest]
fn render_falls_back_when_no_model_cache() {
    // Given a session with a cached context size but no model cache at all.
    use crate::feat::session::token_stats::TokenRecord;
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    state.active_session_mut().push_token_record(TokenRecord {
        model_used: None,
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 5000,
        tokens_received: 0,
        cost: None,
    });
    state.active_session_mut().set_context_size(5000);
    // No model cache.
    assert!(state.provider.model_cache.is_none());

    let (mut terminal, area) = setup_term(80, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 80);
    // Then the status bar shows usage with unknown limit (no model cache at all).
    assert!(
        row.contains("5.0k/???"),
        "expected 5.0k/??? fallback, got: {row}"
    );
}

// --- Context display: (None, Some) case ---

#[rstest::rstest]
fn render_shows_zero_percent_with_max_when_no_messages_sent() {
    // Given a model with a known context length but no messages sent (no context_size).
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state.active_session_mut().set_model(ModelSelection::Single(
        "openrouter/anthropic/claude-sonnet-4".to_owned(),
    ));

    // And a model cache with context_length for the active model.
    let mut cache_entries = std::collections::HashMap::new();
    cache_entries.insert(
        "openrouter".to_owned(),
        vec![crate::feat::provider_infra::ModelInfo {
            id: "anthropic/claude-sonnet-4".to_owned(),
            context_length: Some(200_000),
        }],
    );
    state.provider.model_cache = Some(crate::feat::provider_infra::ModelCache {
        entries: cache_entries,
        last_updated_at: None,
    });

    let (mut terminal, area) = setup_term(100, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 100);
    // Then the status bar shows 0.0% with the real max.
    assert!(row.contains("0.0%/200k"), "expected 0.0%/200k, got: {row}");
}

#[rstest::rstest]
fn render_shows_used_over_unknown_when_no_context_length() {
    // Given a session with context_size but no context_length in the model cache.
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    state.active_session_mut().set_context_size(15_000);

    // Model cache exists but has no context_length.
    let mut cache_entries = std::collections::HashMap::new();
    cache_entries.insert(
        "ollama".to_owned(),
        vec![crate::feat::provider_infra::ModelInfo {
            id: "llama3".to_owned(),
            context_length: None,
        }],
    );
    state.provider.model_cache = Some(crate::feat::provider_infra::ModelCache {
        entries: cache_entries,
        last_updated_at: None,
    });

    let (mut terminal, area) = setup_term(100, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 100);
    // Then the status bar shows the formatted usage with unknown limit.
    assert!(row.contains("15.0k/???"), "expected 15.0k/???, got: {row}");
}

// --- Cost display tests ---

#[rstest::rstest]
fn render_always_shows_cost_even_when_zero() {
    // Given a state with no token records and no history.
    let mut element = StatusBarElement;
    let state = AppState::default();
    let (mut terminal, area) = setup_term(80, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 80);
    // Then cost is always shown as $0.00000.
    assert!(
        row.contains("$0.00000"),
        "expected $0.00000 in status bar, got: {row}"
    );
}

#[rstest::rstest]
fn render_shows_cost_with_non_zero_value() {
    // Given a session with a token record that has cost data.
    use crate::feat::session::token_stats::TokenRecord;
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    state.active_session_mut().push_token_record(TokenRecord {
        model_used: None,
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 1500,
        tokens_received: 750,
        cost: Some(0.00230),
    });
    let (mut terminal, area) = setup_term(80, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 80);
    // Then the status bar shows the cost value.
    assert!(
        row.contains("$0.00230"),
        "expected $0.00230 in status bar, got: {row}"
    );
}

#[rstest::rstest]
fn render_shows_cost_before_turns_indicator() {
    // Given a state with history entries producing turns and a token record with cost.
    use crate::feat::session::token_stats::TokenRecord;
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    state
        .active_session_mut()
        .push_entry(crate::protocol::ChatEntry::user("hello"));
    state
        .active_session_mut()
        .push_entry(crate::protocol::ChatEntry::assistant("hi there"));
    state.active_session_mut().push_token_record(TokenRecord {
        model_used: None,
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 1000,
        tokens_received: 500,
        cost: Some(0.00150),
    });
    let (mut terminal, area) = setup_term(80, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 80);
    // Then cost appears before Turns in the rendered row.
    let cost_pos = row.find("$0.00150").expect("cost should be present");
    let turns_pos = row
        .find("\u{21BB}")
        .expect("turns symbol should be present");
    assert!(
        cost_pos < turns_pos,
        "cost should appear before turns symbol, got: {row}"
    );
}

// --- Tree aggregate display tests ---

#[rstest::rstest]
fn render_hides_tree_aggregate_for_single_session() {
    // Given a single session (no tree).
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    let (mut terminal, area) = setup_term(120, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row0 = buffer_row(&buffer, 0, 120);
    // Then line 1 should NOT contain the tree prefix.
    assert!(
        !row0.contains('\u{1F333}'),
        "single session should not show tree aggregate, got: {row0}"
    );
}

#[rstest::rstest]
fn render_shows_tree_aggregate_when_parent_has_child() {
    // Given a parent session with a child session.
    use crate::feat::session::token_stats::TokenRecord;

    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));

    // Add token records to the active (parent) session.
    state.active_session_mut().push_token_record(TokenRecord {
        model_used: None,
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 1000,
        tokens_received: 500,
        cost: Some(0.01),
    });

    // Create a child session.
    let child_id = crate::protocol::SessionId::new();
    let active_id = state.session.active_session_id().clone();
    {
        let child = state.session_mut_or_create(&child_id);
        child.push_token_record(TokenRecord {
            model_used: None,
            timestamp: jiff::Timestamp::now(),
            tokens_sent: 500,
            tokens_received: 250,
            cost: Some(0.005),
        });
        child.set_parent_session(active_id);
    }

    let (mut terminal, area) = setup_term(120, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row0 = buffer_row(&buffer, 0, 120);
    // Then line 1 right should show tree aggregate with \u{29C9}2.
    assert!(
        row0.contains("\u{29C9}2"),
        "tree aggregate should show session count \u{29C9}2, got: {row0}"
    );
    // And the tree prefix \u{1F333} should be present.
    assert!(
        row0.contains('\u{1F333}'),
        "tree prefix should be present, got: {row0}"
    );
}

#[rstest::rstest]
fn render_shows_tree_aggregate_from_child_viewpoint() {
    // Given a parent with a child, viewing from the child.
    let mut element = StatusBarElement;
    let mut state = AppState::default();

    // Create parent session first.
    let parent_id = crate::protocol::SessionId::new();
    {
        let parent = state.session_mut_or_create(&parent_id);
        parent.push_entry(crate::protocol::ChatEntry::user("parent msg"));
    }

    // Create child session.
    let child_id = crate::protocol::SessionId::new();
    {
        let child = state.session_mut_or_create(&child_id);
        child.push_entry(crate::protocol::ChatEntry::user("child msg"));
        child.set_parent_session(parent_id.clone());
    }

    // Switch to child as active.
    state.session.set_active(child_id);
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));

    let (mut terminal, area) = setup_term(120, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row0 = buffer_row(&buffer, 0, 120);
    // Then tree aggregate still shows \u{29C9}2 (both sessions in tree).
    assert!(
        row0.contains("\u{29C9}2"),
        "tree aggregate from child should show \u{29C9}2, got: {row0}"
    );
}

// --- Alloy display tests ---

#[rstest::rstest]
fn render_single_model_shows_provider_and_model_without_alloy_prefix() {
    // Given a single model selection.
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));

    // When rendering.
    let (mut terminal, area) = setup_term(50, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 50);

    // Then the model shows (provider)/model without alloy prefix.
    assert!(
        row.contains("(ollama)/llama3"),
        "should contain (ollama)/llama3, got: {row}"
    );
    assert!(
        !row.contains("[alloy"),
        "single model should not have alloy prefix, got: {row}"
    );
}

#[rstest::rstest]
fn render_alloy_with_token_records_shows_prefix_and_last_dispatched_model() {
    // Given an alloy with 3 models and a token record showing model-2 as last dispatched.
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state.active_session_mut().set_model(ModelSelection::Alloy {
        models: vec![
            "provider-a/model-1".to_owned(),
            "provider-b/model-2".to_owned(),
            "provider-c/model-3".to_owned(),
        ],
        strategy: AlloyStrategy::RoundRobin { index: 0 },
    });
    state.active_session_mut().push_token_record(TokenRecord {
        timestamp: jiff::Timestamp::now(),
        tokens_sent: 100,
        tokens_received: 50,
        cost: None,
        model_used: Some("provider-b/model-2".to_owned()),
    });

    // When rendering.
    let (mut terminal, area) = setup_term(60, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 60);

    // Then the model shows [alloy 3] prefix with the last-dispatched model.
    assert!(
        row.contains("[alloy 3] (provider-b)/model-2"),
        "should contain [alloy 3] (provider-b)/model-2, got: {row}"
    );
}

#[rstest::rstest]
fn render_alloy_with_no_token_records_falls_back_to_first_model() {
    // Given an alloy with no LLM calls yet.
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state.active_session_mut().set_model(ModelSelection::Alloy {
        models: vec![
            "provider-a/model-1".to_owned(),
            "provider-b/model-2".to_owned(),
        ],
        strategy: AlloyStrategy::RoundRobin { index: 0 },
    });

    // When rendering.
    let (mut terminal, area) = setup_term(50, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 50);

    // Then the model shows [alloy 2] prefix with the first model as fallback.
    assert!(
        row.contains("[alloy 2] (provider-a)/model-1"),
        "should contain [alloy 2] (provider-a)/model-1, got: {row}"
    );
}

#[rstest::rstest]
fn render_alloy_with_one_model_shows_alloy_1() {
    // Given a 1-model alloy.
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state.active_session_mut().set_model(ModelSelection::Alloy {
        models: vec!["ollama/llama3".to_owned()],
        strategy: AlloyStrategy::RoundRobin { index: 0 },
    });

    // When rendering.
    let (mut terminal, area) = setup_term(50, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 50);

    // Then the model shows [alloy 1] prefix.
    assert!(
        row.contains("[alloy 1] (ollama)/llama3"),
        "should contain [alloy 1] (ollama)/llama3, got: {row}"
    );
}

#[rstest::rstest]
fn render_appends_resolved_reasoning_effort_after_model() {
    // Given a session with a model and a global reasoning default of High.
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    state.frontend.preferences.reasoning.default_effort = Some(crate::ReasoningEffort::High);

    // When rendering.
    let (mut terminal, area) = setup_term(50, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 50);

    // Then the model line shows [high] immediately after the model.
    assert!(
        row.contains("(ollama)/llama3 [high]"),
        "should contain [high] after model, got: {row}"
    );
}

#[rstest::rstest]
fn render_session_override_beats_global_reasoning_effort() {
    // Given a global default of High but a session override of Low.
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));
    state.frontend.preferences.reasoning.default_effort = Some(crate::ReasoningEffort::High);
    state.active_session_mut().profile_mut().reasoning_effort = Some(crate::ReasoningEffort::Low);

    // When rendering.
    let (mut terminal, area) = setup_term(50, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 50);

    // Then the override wins: shows [low], not [high].
    assert!(
        row.contains("(ollama)/llama3 [low]"),
        "should show session override [low], got: {row}"
    );
    assert!(
        !row.contains("[high]"),
        "should not show global [high] when override is set, got: {row}"
    );
}

#[rstest::rstest]
fn render_omits_reasoning_effort_bracket_when_unresolved() {
    // Given a model but no resolved reasoning effort (no override, no global).
    let mut element = StatusBarElement;
    let mut state = AppState::default();
    state
        .active_session_mut()
        .set_model(ModelSelection::Single("ollama/llama3".to_owned()));

    // When rendering.
    let (mut terminal, area) = setup_term(50, 2);
    terminal
        .draw(|frame| {
            let ctx = RenderCtx::new(&state);
            element.render(frame, area, &ctx);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let row = buffer_row(&buffer, 1, 50);

    // Then no reasoning bracket appears at all (no [none] noise).
    assert!(
        !row.contains('['),
        "should contain no bracket when effort unresolved, got: {row}"
    );
}
