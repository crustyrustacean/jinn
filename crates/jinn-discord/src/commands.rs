//! Slash commands for the Discord bot.
//!
//! Currently just [`new`] — the session-creation wizard that runs a configured
//! lifecycle setup. Plain messages are handled in [`crate::gateway`] via the
//! poise event handler, not here.

use std::time::Duration;

use jinn_domain::feat::discord::repo_basename;
use jinn_domain::protocol::Intent;
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
        list.push_str(&format!("{}. {}\n", i + 1, p.path.display()));
    }
    list.push_str("\nReply with a number.");
    ctx.say(list).await?;

    // 3. Wait for the user's number.
    let channel = ctx.channel_id();
    let author = ctx.author().id;
    let sctx = ctx.serenity_context();
    let pick = serenity::MessageCollector::new(sctx)
        .channel_id(channel)
        .author_id(author)
        .timeout(Duration::from_secs(120))
        .await;
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
    let chosen = &projects[idx];

    // 4. Ask for branch name.
    ctx.say("Enter branch name:").await?;
    let branch_msg = serenity::MessageCollector::new(sctx)
        .channel_id(channel)
        .author_id(author)
        .timeout(Duration::from_secs(120))
        .await;
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
        ctx.say("No `lifecycle` configured under `[discord]` in jinn.toml.").await?;
        return Ok(());
    };
    let repo = repo_basename(&chosen.path.to_string_lossy()).to_owned();
    let args = vec![branch, repo];

    let new_session_id = {
        let mut state = data.state.write();
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
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    if let Err(e) = data
        .thread_map
        .set(&thread_id, &new_session_id.to_string(), guild_id.as_deref(), now)
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
