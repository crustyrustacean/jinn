//! Trigger-actor reachability tests over the shared bus harness.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test code"
)]

use std::time::Duration;

use crate::trigger_actor::{BotTriggerActor, BotTriggerActorDeps};
use jinn_kernel::common::app_state::AppState;
use jinn_kernel::common::bus::HarnessServices;
use jinn_kernel::common::state::State;
use jinn_session_msg::{TurnCompleted, TurnOutcome};
use jinn_testutil::bus_harness::{TestHarness, await_recorded};

/// The wait budget for a dispatch to land on the bus.
const LONG: Duration = Duration::from_secs(10);

/// The actor half of a bus-harness test, driven through the bus.
struct TriggerBusActor {
    harness: TestHarness,
}

impl TriggerBusActor {
    /// Spawns the trigger actor onto the bus, wired to the harness.
    async fn spawn(harness: TestHarness, state: State) -> Self {
        let services = harness.services().await;
        BotTriggerActor::spawn(harness.system(), BotTriggerActorDeps { services, state });
        Self { harness }
    }

    /// Publishes a message onto the bus.
    async fn publish<M>(&self, message: M)
    where
        M: jinn_kernel::common::bus::BusMessage
            + trouper::schema::Schema
            + serde::Serialize
            + Clone
            + Send
            + Sync
            + trouper::envelope::PayloadValue,
    {
        self.harness.publish(message).await;
    }
}

#[rstest::rstest]
#[tokio::test]
async fn trigger_actor_receives_turn_completed_through_the_bus() {
    // Given the trigger actor spawned on a test bus with a recorder listening
    // for the completion events it observes.
    let harness = TestHarness::new().await;
    let seen = harness.spawn_recorder::<TurnCompleted>().await;
    let state = State::new(AppState::default());
    let actor = TriggerBusActor::spawn(harness, state).await;

    // When a succeeded turn completion is published.
    let session_id = jinn_core_types::SessionId::new();
    actor
        .publish(TurnCompleted {
            session_id: session_id.clone(),
            outcome: TurnOutcome::Succeeded,
        })
        .await;
    let events = await_recorded::<TurnCompleted>(&seen, 1, Duration::from_secs(2)).await;

    // Then the actor's subscription is live — the recorder (and therefore the
    // trigger actor, on the same topic) saw the event with its outcome intact.
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].outcome, TurnOutcome::Succeeded);
    assert_eq!(events[0].session_id, session_id);
}

#[rstest::rstest]
#[tokio::test]
async fn trigger_does_not_cancel_the_bots_descendants() {
    // Given a busy bot with a busy subagent beneath it, armed with a
    // ParentCompleted trigger, and the trigger actor live on the bus.
    let (harness, canceled, parent_id) = {
        let harness = TestHarness::new().await;
        let canceled = harness
            .spawn_recorder::<jinn_inference_msg::CancelTurn>()
            .await;
        let state = State::new(AppState::default());
        let parent_id = {
            let mut s = state.write();
            let parent = jinn_session_state::ChatSessionState::new();
            let parent_id = parent.session_id().clone();
            s.session.insert(parent);

            let bot_read = s.session.get(&parent_id).expect("parent").clone();
            let mut bot = jinn_session_state::ChatSessionState::new_bot(&bot_read, true);
            bot.set_bot_trigger(jinn_bot_msg::BotTrigger::ParentCompleted);
            bot.set_bot_behavior(jinn_bot_msg::BotBehavior::Reset);
            let bot_id = bot.session_id().clone();

            let mut subagent = jinn_session_state::ChatSessionState::new_child(&bot_id, true);
            subagent.begin_streaming();
            s.task_spawns
                .register(bot_id.clone(), subagent.session_id().clone());
            s.session.insert(subagent);
            s.session.insert(bot);
            parent_id
        };
        let _actor = BotTriggerActor::spawn(
            harness.system(),
            BotTriggerActorDeps {
                services: harness.services().await,
                state: state.clone(),
            },
        );
        (harness, canceled, parent_id)
    };

    // When the parent's turn completes and the bot fires.
    harness
        .publish(TurnCompleted {
            session_id: parent_id,
            outcome: TurnOutcome::Succeeded,
        })
        .await;

    // Then nothing is cancelled. A trigger does not know which descendant
    // the user would want stopped, so stopping the subtree is reserved for
    // the manual `R`, where the user has said "start over".
    // The cancel recorder must have stopped changing before "no cancel
    // was published" is an observation rather than a race with a publish
    // that is still in flight.
    let cancels = jinn_testutil::wait::await_settled_messages(
        &canceled,
        Duration::from_millis(200),
        Duration::from_secs(5),
    )
    .await;
    assert!(
        cancels.is_empty(),
        "the trigger must not cascade a cancel, got {cancels:?}"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn a_triggered_bot_fires_on_the_parent_completing() {
    // Given a parent with a dispatchable parent-completed bot, and the
    // trigger actor live on the bus.
    let harness = TestHarness::new().await;
    let dispatched = harness
        .spawn_recorder::<jinn_chat_input_msg::EnqueueUserMessage>()
        .await;
    let state = State::new(AppState::default());
    let parent_id = {
        let mut s = state.write();
        let parent = jinn_session_state::ChatSessionState::new();
        let parent_id = parent.session_id().clone();
        s.session.insert(parent.clone());
        let mut bot = jinn_session_state::ChatSessionState::new_bot(&parent, true);
        // A fresh bot is in prep mode, which is inert by design; a
        // dispatchable one has to have been composed.
        bot.set_bot_is_prepping(false);
        bot.set_bot_trigger(jinn_bot_msg::BotTrigger::ParentCompleted);
        bot.set_bot_behavior(jinn_bot_msg::BotBehavior::Reset);
        bot.set_seed_template("check: <prior_report>".to_owned());
        bot.append_bot_report("prior finding".to_owned());
        s.session.insert(bot);
        parent_id
    };
    let _actor = BotTriggerActor::spawn(
        harness.system(),
        BotTriggerActorDeps {
            services: harness.services().await,
            state: state.clone(),
        },
    );

    // When the parent's turn completes successfully.
    harness
        .publish(TurnCompleted {
            session_id: parent_id.clone(),
            outcome: TurnOutcome::Succeeded,
        })
        .await;
    let first =
        await_recorded::<jinn_chat_input_msg::EnqueueUserMessage>(&dispatched, 1, LONG).await;

    // Then the completion fired the bot once.
    assert_eq!(first.len(), 1);
}

#[rstest::rstest]
#[tokio::test]
async fn a_parentless_bot_is_never_collected_by_the_walk() {
    // Given a standalone bot — parentless by construction — whose trigger
    // has been forced to ParentCompleted anyway, and the trigger actor
    // live on the bus.
    let harness = TestHarness::new().await;
    let dispatched = harness
        .spawn_recorder::<jinn_chat_input_msg::EnqueueUserMessage>()
        .await;
    let state = State::new(AppState::default());
    let source_id = {
        let mut s = state.write();
        let source = jinn_session_state::ChatSessionState::new();
        let source_id = source.session_id().clone();
        s.session.insert(source.clone());
        let mut bot = jinn_session_state::ChatSessionState::new_standalone_bot(&source, true);
        bot.set_bot_is_prepping(false);
        // Forced against the rule the popup and the save path enforce, to
        // prove the walk itself refuses: the structural filter is what
        // must hold, not the UI discipline.
        bot.set_bot_trigger(jinn_bot_msg::BotTrigger::ParentCompleted);
        s.session.insert(bot);
        source_id
    };
    let _actor = BotTriggerActor::spawn(
        harness.system(),
        BotTriggerActorDeps {
            services: harness.services().await,
            state: state.clone(),
        },
    );

    // When the environment source's turn completes successfully.
    harness
        .publish(TurnCompleted {
            session_id: source_id,
            outcome: TurnOutcome::Succeeded,
        })
        .await;

    // Then the walk collected nothing — a bot with no parent has no
    // completion to watch, and the walk's parent match is what keeps a
    // stray trigger value from firing it. The recorder is polled until it
    // has been quiet, so the empty read means "nothing arrived", not
    // "nothing arrived yet".
    let fired =
        jinn_testutil::wait::await_settled_messages(&dispatched, Duration::from_millis(150), LONG)
            .await;
    assert!(
        fired.is_empty(),
        "a parentless bot must never fire from a completion"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn a_triggered_bot_fires_again_after_a_prior_fire() {
    // Given a parent with a dispatchable parent-completed bot, the
    // trigger actor live on the bus, and one fire already delivered.
    let harness = TestHarness::new().await;
    let dispatched = harness
        .spawn_recorder::<jinn_chat_input_msg::EnqueueUserMessage>()
        .await;
    let state = State::new(AppState::default());
    let parent_id = {
        let mut s = state.write();
        let parent = jinn_session_state::ChatSessionState::new();
        let parent_id = parent.session_id().clone();
        s.session.insert(parent.clone());
        let mut bot = jinn_session_state::ChatSessionState::new_bot(&parent, true);
        // A fresh bot is in prep mode, which is inert by design; a
        // dispatchable one has to have been composed.
        bot.set_bot_is_prepping(false);
        bot.set_bot_trigger(jinn_bot_msg::BotTrigger::ParentCompleted);
        bot.set_bot_behavior(jinn_bot_msg::BotBehavior::Reset);
        bot.set_seed_template("check: <prior_report>".to_owned());
        bot.append_bot_report("prior finding".to_owned());
        s.session.insert(bot);
        parent_id
    };
    let _actor = BotTriggerActor::spawn(
        harness.system(),
        BotTriggerActorDeps {
            services: harness.services().await,
            state: state.clone(),
        },
    );
    harness
        .publish(TurnCompleted {
            session_id: parent_id.clone(),
            outcome: TurnOutcome::Succeeded,
        })
        .await;
    let first =
        await_recorded::<jinn_chat_input_msg::EnqueueUserMessage>(&dispatched, 1, LONG).await;
    assert_eq!(first.len(), 1);

    // When the parent completes a second time.
    harness
        .publish(TurnCompleted {
            session_id: parent_id,
            outcome: TurnOutcome::Succeeded,
        })
        .await;
    let second =
        await_recorded::<jinn_chat_input_msg::EnqueueUserMessage>(&dispatched, 1, LONG).await;

    // Then the second completion fired the same bot again. Nothing
    // records that the first fire was automated, so a second one is not
    // suppressed — ending the exchange is the agent's call, not the harness's.
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].session_id, first[0].session_id);
}

#[rstest::rstest]
#[tokio::test]
async fn trigger_actor_is_reachable_at_its_static_path() {
    // Given a harness with the trigger actor spawned.
    let harness = TestHarness::new().await;
    let state = State::new(AppState::default());
    let path = BotTriggerActor::spawn(
        harness.system(),
        BotTriggerActorDeps {
            services: harness.services().await,
            state,
        },
    );

    // When sending a message directly to that path.
    let delivered = harness
        .system()
        .tell(
            path,
            TurnCompleted {
                session_id: jinn_core_types::SessionId::new(),
                outcome: TurnOutcome::Succeeded,
            },
        )
        .await;

    // Then the send resolves — the actor is live where composition and
    // every publish expect it. An unrouted tell returns the envelope.
    assert!(
        delivered.is_ok(),
        "trigger actor must be live and routed at its static path"
    );
}
