//! Slash commands for the Discord bot.
//!
//! [`new`] runs the session-creation wizard (lifecycle setup); [`teardown`] and
//! [`archive`] operate on the session bound to the invoking thread. Plain
//! messages are handled in [`crate::gateway`] via the poise event handler, not here.

use std::time::Duration;

use jinn_domain::feat::discord::repo_basename;
use jinn_domain::feat::session::protocol::archive_session::ArchiveSession;
use jinn_domain::protocol::Intent;
use jinn_domain::{Bridge, SessionId};
use poise::serenity_prelude as serenity;

use crate::gateway::{BotContext, BotError};

/// Start a new jinn session in this thread.
///
/// Runs the configured lifecycle setup with two positional args: the branch
/// name (from the user) and the repo basename (derived from the chosen
/// project's path). The setup result is posted by the drain loop when the
/// `SessionSetupCompleted` event arrives.
#[poise::command(slash_command)]
pub async fn new(ctx: BotContext<'_>) -> Result<(), BotError> {
    let data = ctx.data();

    // 1. Gather projects from preferences.
    let projects = {
        let state = data.state.read();
        state.frontend.preferences.projects.clone()
    };
    if projects.is_empty() {
        ctx.say("No projects configured. Add `[[project]]` entries to jinn.toml first.")
            .await?;
        return Ok(());
    }

    // 2. Present numbered list.
    let mut list = String::from("Available projects:\n");
    for (i, p) in projects.iter().enumerate() {
        use std::fmt::Write as _;
        let _ = writeln!(list, "{}. {}", i + 1, p.path.display());
    }
    list.push_str("\nReply with a number.");
    ctx.say(list).await?;

    // 3. Wait for the user's number.
    let channel = ctx.channel_id();
    let author = ctx.author().id;
    let sctx = ctx.serenity_context();
    let pick = collect_reply(sctx, channel, author).await;
    let Some(pick) = pick else {
        ctx.say("Timed out waiting for project selection.").await?;
        return Ok(());
    };

    let idx: usize = match pick.content.trim().parse::<usize>() {
        Ok(n) if (1..=projects.len()).contains(&n) => n - 1,
        _ => {
            ctx.say("Invalid selection. Run `/new` again.").await?;
            return Ok(());
        }
    };
    let Some(chosen) = projects.get(idx) else {
        ctx.say("Invalid selection. Run `/new` again.").await?;
        return Ok(());
    };

    // 4. Ask for branch name.
    ctx.say("Enter branch name:").await?;
    let branch_msg = collect_reply(sctx, channel, author).await;
    let Some(branch_msg) = branch_msg else {
        ctx.say("Timed out waiting for branch name.").await?;
        return Ok(());
    };
    let branch = branch_msg.content.trim().to_owned();
    if branch.is_empty() {
        ctx.say("Empty branch name. Run `/new` again.").await?;
        return Ok(());
    }

    // 5. Run the lifecycle setup via the intent handler. The lifecycle name
    //    is required to use the bot — enforced by config validation, but we
    //    guard here too so a misconfigured instance fails gracefully.
    let Some(lifecycle) = data.config.lifecycle.clone() else {
        ctx.say("No `lifecycle` configured under `[discord]` in jinn.toml.")
            .await?;
        return Ok(());
    };
    let repo = repo_basename(&chosen.path.to_string_lossy()).to_owned();
    let args = vec![branch, repo];

    let new_session_id = {
        let mut state = data.state.write();
        // Stash the chosen project's path so the lifecycle handler consumes it
        // as the new session's starting CWD (same convention as the project
        // picker). Without this the handler falls back to inheriting the
        // currently-active session's CWD, which is unrelated to the pick.
        state.frontend.pending_session_cwd = Some(chosen.path.clone());
        let result = jinn_domain::feat::intent::IntentHandler::handle(
            &Intent::SessionLifecycleSetup {
                lifecycle_name: lifecycle.clone(),
                args: args.clone(),
            },
            &mut state,
            None,
        );
        for closure in result.messages {
            let _ = data.bridge.send(closure);
        }
        state.session.active_session_id().clone()
    };

    // 6. Record the thread→session mapping so future messages + the drain loop
    //    can find the session.
    let thread_id = channel.get().to_string();
    let guild_id = ctx.guild_id().map(|g| g.get().to_string());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    if let Err(e) = data
        .thread_map
        .set(
            &thread_id,
            &new_session_id.to_string(),
            guild_id.as_deref(),
            now,
        )
        .await
    {
        tracing::warn!(error = ?e, "failed to record thread→session mapping");
    }

    // 7. The actual "Setup complete" message is posted by the drain loop when
    //    the SessionSetupCompleted event arrives. Acknowledge here only.
    ctx.say(format!(
        "Setting up session `{new_session_id}` (lifecycle `{lifecycle}`)..."
    ))
    .await?;
    Ok(())
}

/// Re-run the lifecycle teardown script for the session bound to this thread.
///
/// Looks up the thread→session mapping, renders the bound session's teardown
/// command (replaying its stored lifecycle args), and publishes
/// `RunSessionTeardown`. The ✅/❌ result is posted by the drain loop when the
/// `SessionTeardownFinished` event arrives.
///
/// Replies with an error message when no session is bound to the thread, or
/// when the session has no teardown command configured.
#[poise::command(slash_command)]
pub async fn teardown(ctx: BotContext<'_>) -> Result<(), BotError> {
    let data = ctx.data().clone();
    let thread_id = ctx.channel_id().get().to_string();

    let Some(session_id_str) = resolve_bound_session(&data, &thread_id).await? else {
        ctx.say("No session bound to this thread. Use `/new` first.")
            .await?;
        return Ok(());
    };
    let session_id = SessionId::from(session_id_str);

    // Resolve + render under a short-lived read guard so it never spans an await.
    let Some(publish) = build_teardown_publish(&data.state.read(), &session_id) else {
        ctx.say("This session has no teardown command configured.")
            .await?;
        return Ok(());
    };

    data.bridge.send(publish)?;
    ctx.say(format!("Running teardown for session `{session_id}`…"))
        .await?;
    Ok(())
}

/// Resolve + render the teardown command for `session_id` and wrap it as a
/// publish closure. Returns `None` when the session has no teardown command.
fn build_teardown_publish(
    state: &jinn_domain::StateReadGuard<'_>,
    session_id: &SessionId,
) -> Option<jinn_domain::BridgeClosure> {
    let msg = jinn_domain::feat::session_lifecycle::intent::build_run_session_teardown(state, session_id)?;
    Some(Bridge::publish_closure(msg))
}

/// Archive the session bound to this thread without running teardown.
///
/// Looks up the thread→session mapping and publishes `ArchiveSession`, which
/// marks the session archived in SQLite and removes it from memory. The ✅
/// confirmation is posted by the drain loop when the `SessionArchived` event
/// arrives.
///
/// Replies with an error message when no session is bound to the thread.
#[poise::command(slash_command)]
pub async fn archive(ctx: BotContext<'_>) -> Result<(), BotError> {
    let data = ctx.data().clone();
    let thread_id = ctx.channel_id().get().to_string();

    let Some(session_id_str) = resolve_bound_session(&data, &thread_id).await? else {
        ctx.say("No session bound to this thread. Use `/new` first.")
            .await?;
        return Ok(());
    };
    let session_id = SessionId::from(session_id_str);

    data.bridge
        .send(Bridge::publish_closure(ArchiveSession { session_id: session_id.clone() }))?;
    ctx.say(format!("Archiving session `{session_id}`…")).await?;
    Ok(())
}

/// Resolve the jinn session bound to a Discord thread, if any.
///
/// Returns `Ok(None)` when the thread has no bound session (the caller replies
/// with a helpful message). Errors only on a DAO read failure.
///
/// # Errors
///
/// Returns [`BotError`] if the thread-map lookup fails.
async fn resolve_bound_session(
    data: &crate::gateway::BotData,
    thread_id: &str,
) -> Result<Option<String>, BotError> {
    data.thread_map
        .get_session_by_thread(thread_id)
        .await
        .map_err(|e| Box::from(format!("thread lookup failed: {e:?}")) as BotError)
}

/// Collect one message from the user who invoked the command, within a
/// 2-minute window.
#[expect(
    clippy::duration_suboptimal_units,
    reason = "from_mins is unstable; from_secs is the stable alternative"
)]
async fn collect_reply(
    sctx: &serenity::Context,
    channel: serenity::ChannelId,
    author: serenity::UserId,
) -> Option<serenity::Message> {
    serenity::MessageCollector::new(sctx)
        .channel_id(channel)
        .author_id(author)
        .timeout(Duration::from_secs(2 * 60))
        .await
}
