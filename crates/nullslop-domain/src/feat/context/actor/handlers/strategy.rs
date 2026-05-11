//! Strategy handlers — strategy switching and state restoration.

use crate::common::actor::ActorContext;
use crate::feat::context::protocol::command::{RestoreStrategyState, SwitchPromptStrategy};
use crate::feat::context::protocol::event::{PromptStrategySwitched, StrategyStateUpdated};
use crate::protocol::{Command, Event};

use super::super::PromptAssemblyActor;

impl PromptAssemblyActor {
    /// Handles [`PromptStrategySwitched`] by creating a new strategy via the factory.
    pub(in crate::feat::context::actor) fn on_prompt_strategy_switched(
        &mut self,
        evt: &PromptStrategySwitched,
    ) {
        let Some(factory) = self.factory.as_ref() else {
            tracing::error!("no strategy factory available");
            return;
        };
        match factory.create(&evt.strategy_id) {
            Ok(new_strategy) => {
                self.strategies.insert(evt.session_id.clone(), new_strategy);
            }
            Err(e) => {
                tracing::error!("failed to create strategy '{}': {e:?}", evt.strategy_id);
            }
        }
    }

    /// SwitchPromptStrategy: switch strategy, emit RestoreStrategyState + PromptStrategySwitched.
    pub(in crate::feat::context::actor) fn handle_switch_prompt_strategy(
        &self,
        payload: &SwitchPromptStrategy,
        ctx: &ActorContext,
    ) {
        let blob = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.switch_strategy(payload.strategy_id.clone());
            state
                .context
                .strategy_state
                .get(&(payload.session_id.clone(), payload.strategy_id.clone()))
                .cloned()
                .unwrap_or(serde_json::json!({}))
        };

        if let Err(e) = ctx.send_command(Command::RestoreStrategyState {
            payload: RestoreStrategyState {
                session_id: payload.session_id.clone(),
                strategy_id: payload.strategy_id.clone(),
                blob,
            },
        }) {
            tracing::warn!(err = ?e, "context-actor failed to emit RestoreStrategyState");
        }

        if let Err(e) = ctx.send_event(Event::PromptStrategySwitched {
            payload: PromptStrategySwitched {
                session_id: payload.session_id.clone(),
                strategy_id: payload.strategy_id.clone(),
            },
        }) {
            tracing::warn!(
                err = ?e,
                "context-actor failed to emit PromptStrategySwitched"
            );
        }
    }

    /// RestoreStrategyState: set strategy blob on session, emit StrategyStateUpdated.
    pub(in crate::feat::context::actor) fn handle_restore_strategy_state(
        &self,
        payload: &RestoreStrategyState,
        ctx: &ActorContext,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.set_strategy_state(payload.blob.clone());
            state.context.strategy_state.insert(
                (payload.session_id.clone(), payload.strategy_id.clone()),
                payload.blob.clone(),
            );
        }

        if let Err(e) = ctx.send_event(Event::StrategyStateUpdated {
            payload: StrategyStateUpdated {
                session_id: payload.session_id.clone(),
                strategy_id: payload.strategy_id.clone(),
                blob: payload.blob.clone(),
            },
        }) {
            tracing::warn!(err = ?e, "context-actor failed to emit StrategyStateUpdated");
        }
    }
}
