//! The poise gateway task + `BotData`.
//!
//! This module owns the Discord-side of the bot: a [`poise`] framework running
//! the gateway websocket, slash commands (`/new`), and a plain-message handler.
//! It bridges Discord to the jinn actor bus via the shared [`State`] (read
//! session history) and [`Bridge`] (publish commands).
//!
//! See `.plans/discord/plan.md` for the full architecture.

use std::sync::Arc;

use derive_more::Debug;
use error_stack::Report;
use jinn_domain::feat::chat_input::protocol::command::{EnqueueUserMessage, SubmitSteeringMessage};
use jinn_domain::feat::discord::{
    BridgeEvent, DiscordConfig, DiscordThreadMap, FinalReply, RouteDecision, read_final_reply,
    route_decision, split_message,
};
use jinn_domain::feat::session::chat_entry::ChatEntry;
use jinn_domain::feat::session::protocol::session_load_requested::SessionLoadRequested;
use jinn_domain::{Bridge, State};
use poise::serenity_prelude as serenity;
use wherror::Error;

use crate::commands;

/// Error spawning the Discord gateway.
///
/// Returned when the bot token is missing or the poise framework cannot start.
#[derive(Debug, Error)]
#[error(debug)]
pub struct SpawnError;

pub(crate) type BotError = Box<dyn std::error::Error + Send + Sync>;
pub(crate) type BotContext<'a> = poise::Context<'a, BotData, BotError>;

/// Shared context passed to every poise command and event handler.
///
/// Held behind `Arc<UserData>` by poise. Clones cheaply — every field is either
/// an `Arc`, a `Bridge` (which is `Clone`), or a channel handle.
#[derive(Debug, Clone)]
pub struct BotData {
    /// Shared jinn `State` (sessions, frontend, etc.). The bot reads session
    /// history from here to extract the final reply.
    pub state: State,
    /// Clone of the actor bus — used to publish `EnqueueUserMessage`,
    /// `SubmitSteeringMessage`, `SessionLoadRequested`, etc. via closures.
    pub bridge: Bridge,
    /// DAO over `sessions.db` for thread↔session mapping persistence.
    pub thread_map: DiscordThreadMap,
    /// The bot configuration from `jinn.toml` `[discord]`.
    pub config: Arc<DiscordConfig>,
}

/// Runs the Discord gateway: starts poise, registers slash commands, and
/// spawns the bridge-event drain loop. Blocks the calling task until the
/// gateway shuts down.
///
/// `rx` is the receiving half of the channel fed by `DiscordBridgeActor`; the
/// drain loop consumes [`BridgeEvent`]s and posts the bot's replies to Discord.
///
/// # Errors
///
/// Returns [`SpawnError`] if the bot token is missing/empty or the poise
/// framework fails to start.
pub async fn run(
    data: BotData,
    token: String,
    rx: tokio::sync::mpsc::Receiver<BridgeEvent>,
) -> Result<(), Report<SpawnError>> {
    if token.trim().is_empty() {
        tracing::error!("discord bot enabled but no token configured");
        return Err(Report::new(SpawnError));
    }

    let intents =
        serenity::GatewayIntents::non_privileged() | serenity::GatewayIntents::MESSAGE_CONTENT;

    let drain_data = data.clone();
    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![commands::new()],
            // Plain-message handler: route an inbound Discord message to the
            // jinn session bound to its thread (resuming/un-archiving first),
            // then enqueue or steer depending on the current phase.
            event_handler: |ctx, event, _framework, data| Box::pin(on_event(ctx, event, data)),
            on_error: |error| {
                Box::pin(async move {
                    tracing::error!(?error, "poise framework error");
                })
            },
            ..Default::default()
        })
        .setup(move |ctx, _ready, _framework| {
            let data = data.clone();
            Box::pin(async move {
                // Spawn the bridge-event drain loop. It owns its own BotData clone
                // plus the poise http handle so it can post replies back to Discord
                // without touching the command path.
                let http = ctx.http.clone();
                let drain_data = drain_data.clone();
                tokio::spawn(drain_loop(drain_data, http, rx));
                Ok(data)
            })
        })
        .build();

    let mut client = serenity::ClientBuilder::new(token, intents)
        .framework(framework)
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "failed to build discord client");
            Report::new(SpawnError)
        })?;

    client.start().await.map_err(|e| {
        tracing::error!(error = ?e, "discord gateway exited with error");
        Report::new(SpawnError)
    })?;
    Ok(())
}

/// Consume [`BridgeEvent`]s from the bridge actor and post replies to Discord.
///
/// Two cases:
/// - [`BridgeEvent::SetupCompleted`] — format the result message and post to the
///   thread bound to the session (reverse lookup).
/// - [`BridgeEvent::TurnFinished`] — read the final assistant/error reply from
///   shared state, split it into ≤2000-char chunks, and post each to the thread.
async fn drain_loop(
    data: BotData,
    http: Arc<serenity::Http>,
    mut rx: tokio::sync::mpsc::Receiver<BridgeEvent>,
) {
    while let Some(event) = rx.recv().await {
        match event {
            BridgeEvent::SetupCompleted {
                session_id,
                cwd,
                error,
            } => match resolve_thread(&data, &session_id).await {
                Ok(Some(channel_id)) => {
                    let msg = match &error {
                        Some(e) => format!("❌ Setup failed: {e}"),
                        None => format!("✅ Setup complete ({})", cwd.display()),
                    };
                    if let Err(e) = post_message(&http, channel_id, &msg).await {
                        tracing::warn!(error = ?e, "failed to post setup result");
                    }
                }
                Ok(None) => {
                    tracing::debug!(%session_id, "no thread bound to session; dropping setup result");
                }
                Err(e) => tracing::warn!(error = %e, "thread lookup failed for setup result"),
            },
            BridgeEvent::TurnFinished { session_id } => {
                match resolve_thread(&data, &session_id).await {
                    Ok(Some(channel_id)) => match read_reply(&data, &session_id) {
                        Some(reply) => {
                            if let Err(e) = post_reply(&http, channel_id, &reply).await {
                                tracing::warn!(error = ?e, "failed to post final reply");
                            }
                        }
                        None => {
                            tracing::debug!(%session_id, "turn finished but no final reply found");
                        }
                    },
                    Ok(None) => {
                        tracing::debug!(%session_id, "no thread bound to session; dropping reply");
                    }
                    Err(e) => tracing::warn!(error = %e, "thread lookup failed for final reply"),
                }
            }
        }
    }
    tracing::info!("discord bridge-event drain loop exiting");
}

/// Read the final assistant/error reply for a session from shared state.
fn read_reply(data: &BotData, session_id: &jinn_domain::SessionId) -> Option<FinalReply> {
    let state = data.state.read();
    let session = state.session(session_id);
    read_final_reply(session.history())
}

/// Look up the Discord thread bound to a jinn session (reverse mapping).
async fn resolve_thread(
    data: &BotData,
    session_id: &jinn_domain::SessionId,
) -> Result<Option<serenity::ChannelId>, String> {
    match data
        .thread_map
        .get_thread_by_session(&session_id.to_string())
        .await
    {
        Ok(Some(mapping)) => mapping
            .thread_id
            .parse::<u64>()
            .map(serenity::ChannelId::new)
            .map(Some)
            .map_err(|e| format!("invalid thread id {}: {e}", mapping.thread_id)),
        Ok(None) => Ok(None),
        Err(e) => Err(format!("{e:?}")),
    }
}

/// Post a single plain message to a channel.
async fn post_message(
    http: &serenity::Http,
    channel: serenity::ChannelId,
    body: &str,
) -> Result<(), serenity::Error> {
    channel.say(http, body).await.map(|_| ())
}

/// Split a final reply into ≤2000-char chunks and post each to a channel.
async fn post_reply(
    http: &serenity::Http,
    channel: serenity::ChannelId,
    reply: &FinalReply,
) -> Result<(), serenity::Error> {
    let body = match reply {
        FinalReply::Assistant(t) | FinalReply::Error(t) => t.as_str(),
    };
    for chunk in split_message(body) {
        channel.say(http, chunk).await?;
    }
    Ok(())
}

/// Poise event handler — dispatches plain messages to the bound session.
///
/// Slash commands are routed by poise separately; this only handles the
/// `Message` event for non-bot authors.
async fn on_event(
    ctx: &serenity::Context,
    event: &serenity::FullEvent,
    data: &BotData,
) -> Result<(), BotError> {
    let serenity::FullEvent::Message { new_message } = event else {
        return Ok(());
    };
    // Ignore our own messages.
    if new_message.author.id == ctx.cache.current_user().id {
        return Ok(());
    }
    handle_inbound_message(ctx, new_message, data).await?;
    Ok(())
}

/// Route an inbound Discord message to the jinn session bound to its channel.
///
/// If a session is bound:
/// 1. Fire `SessionLoadRequested` to (re-)load + un-archive it.
/// 2. Read the session's phase, route to enqueue (Idle) or steer (mid-turn).
///
/// If no session is bound, reply with a hint to run `/new`.
async fn handle_inbound_message(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    data: &BotData,
) -> Result<(), BotError> {
    let thread_id = msg.channel_id.get().to_string();
    let Some(session_id) = data
        .thread_map
        .get_session_by_thread(&thread_id)
        .await
        .map_err(|e| format!("thread map lookup: {e:?}"))?
    else {
        msg.reply(
            ctx,
            "No session bound to this thread — run `/new` to start one.",
        )
        .await?;
        return Ok(());
    };

    let session_id: jinn_domain::SessionId = session_id.into();

    // (Re-)load + un-archive the session on every message. Cheap no-op if
    // already loaded; restores archived sessions transparently.
    publish(
        &data.bridge,
        SessionLoadRequested {
            session_id: session_id.clone(),
        },
    );

    let phase = {
        let state = data.state.read();
        state.session(&session_id).phase()
    };

    match route_decision(phase) {
        RouteDecision::Enqueue => publish(
            &data.bridge,
            EnqueueUserMessage {
                session_id: session_id.clone(),
                entry: ChatEntry::user(msg.content.clone()),
            },
        ),
        RouteDecision::Steer => publish(
            &data.bridge,
            SubmitSteeringMessage {
                session_id: session_id.clone(),
                text: msg.content.clone(),
            },
        ),
    }
    Ok(())
}

/// Publish a typed bus message via a bridge closure.
pub(crate) fn publish<M>(bridge: &Bridge, msg: M)
where
    M: Clone + Send + 'static,
{
    if let Err(e) = bridge.send(Bridge::publish_closure(msg)) {
        tracing::error!(error = %e, "bridge send failed");
    }
}

#[cfg(test)]
mod tests;
