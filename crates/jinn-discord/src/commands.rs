//! Slash commands for the Discord bot.
//!
//! [`new`] starts a session via the configured lifecycle wizard; [`prompts`]
//! lists the prompt templates available to the current thread's session.
//! Plain messages are handled in [`crate::gateway`] via the poise event
//! handler, not here.

use std::time::Duration;

use jinn_domain::feat::context::prompt_template::PromptTemplateStore;
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
/// Outcome of a single locked read of a session's prompt store.
///
/// Captured under one read lock so the three outcomes are decided from a
/// consistent state snapshot — never split across two lock passes.
enum Lookup {
    /// The session has a non-empty store; here is the rendered cheat-sheet.
    List(String),
    /// The session exists but its prompt store is empty.
    Empty,
    /// The session is absent from state (mid-restore race).
    Missing,
}

/// List the prompt templates available to this thread's session.
///
/// Resolves the channel → session mapping via the thread map, reads the
/// session's already-merged prompt-template store, and replies privately
/// (ephemerally) with each prompt's `#name` and description. Composition is
/// unchanged — the user still types `#name` into a normal message. No rescan
/// is triggered; the store is whatever the session loaded at startup.
#[poise::command(slash_command)]
pub async fn prompts(ctx: BotContext<'_>) -> Result<(), BotError> {
    let data = ctx.data();
    let channel_id = ctx.channel_id();

    // 1. Resolve channel → session via the thread map (same lookup as the
    //    inbound message handler). Unbound channels get an ephemeral hint
    //    rather than the silent no-op used on the message path.
    let thread_id = channel_id.get().to_string();
    let reply = match data.thread_map.get_session_by_thread(&thread_id).await {
        Ok(Some(id)) => {
            let session_id: jinn_domain::SessionId = id.into();
            // 2. Read the session's prompt store under a single short-lived
            //    read lock. The decision (list / empty / missing) is captured
            //    as a `Lookup` so the guard is dropped before the await on
            //    ctx.send below — and so the three outcomes are decided from
            //    one consistent state snapshot rather than two lock passes.
            let lookup = {
                let state = data.state.read();
                match state.try_session(&session_id) {
                    Some(session) => {
                        match render_prompts_list(session.discovered_prompt_templates()) {
                            Some(text) => Lookup::List(text),
                            None => Lookup::Empty,
                        }
                    }
                    None => Lookup::Missing,
                }
            };
            match lookup {
                Lookup::List(text) => ephemeral(text),
                Lookup::Empty => ephemeral("No prompts configured for this session."),
                Lookup::Missing => ephemeral("Session restoring — please try again shortly."),
            }
        }
        Ok(None) => ephemeral("No active session in this thread — run `/new` first."),
        Err(e) => ephemeral(format!("Failed to look up session: {e:?}")),
    };

    ctx.send(reply).await?;
    Ok(())
}

/// Build an ephemeral `CreateReply` carrying the given content.
fn ephemeral(content: impl Into<String>) -> poise::CreateReply {
    poise::CreateReply::default()
        .content(content)
        .ephemeral(true)
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

/// Soft cap on the rendered cheat-sheet length, leaving headroom under
/// Discord's 2000-character message body limit. Prompts beyond this are
/// truncated with a count rather than causing `ctx.send` to error.
const PROMPT_LIST_SOFT_LIMIT: usize = 1900;

/// Renders a prompt-template store as a human-readable cheat-sheet of
/// `#name` tokens and their descriptions.
///
/// Returns `None` when the store is empty so the caller can emit a distinct
/// "no prompts configured" message. The store's slice order is preserved
/// so the listing is stable and deterministic. If the full list would
/// approach Discord's message-length limit, rendering stops early and a
/// trailing count of omitted prompts is appended.
fn render_prompts_list(store: &PromptTemplateStore) -> Option<String> {
    let templates = store.templates();
    if templates.is_empty() {
        return None;
    }
    let mut out = String::from("Available prompts (use `#name` in a message):\n");
    let mut shown = 0;
    for t in templates {
        let line = format!("`#{}` — {}\n", t.name, t.description);
        if out.len() + line.len() > PROMPT_LIST_SOFT_LIMIT {
            break;
        }
        out.push_str(&line);
        shown += 1;
    }
    let remaining = templates.len() - shown;
    if remaining > 0 {
        use std::fmt::Write as _;
        let _ = write!(
            out,
            "\n… and {remaining} more not shown (message length limit)."
        );
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::render_prompts_list;
    use jinn_domain::feat::context::prompt_template::PromptTemplateStore;
    use jinn_domain::protocol::PromptTemplate;

    fn template(name: &str, description: &str) -> PromptTemplate {
        PromptTemplate {
            name: name.to_owned(),
            description: description.to_owned(),
            body: String::new(),
        }
    }

    #[test]
    #[expect(clippy::expect_used, reason = "non-empty store is constructed above")]
    fn render_prompts_list_formats_name_and_description() {
        // Given a store with two prompts.
        let store = PromptTemplateStore::from_vec(vec![
            template("code-review", "Perform a thorough code review"),
            template("summarize", "Summarize the conversation"),
        ]);

        // When rendering.
        let out = render_prompts_list(&store).expect("non-empty store renders");

        // Then each prompt appears as `#name` — description.
        assert!(
            out.contains("`#code-review` — Perform a thorough code review"),
            "missing code-review line: {out:?}"
        );
        assert!(
            out.contains("`#summarize` — Summarize the conversation"),
            "missing summarize line: {out:?}"
        );
    }

    #[test]
    fn render_prompts_list_empty_store_returns_none() {
        // Given an empty store.
        let store = PromptTemplateStore::from_vec(vec![]);

        // When rendering.
        let out = render_prompts_list(&store);

        // Then no text is produced.
        assert!(out.is_none());
    }

    #[test]
    #[expect(
        clippy::expect_used,
        reason = "store order and presence are the behavior under test"
    )]
    fn render_prompts_list_preserves_store_order() {
        // Given a store with a known insertion order.
        let store = PromptTemplateStore::from_vec(vec![
            template("first", "alpha"),
            template("second", "beta"),
            template("third", "gamma"),
        ]);

        // When rendering.
        let out = render_prompts_list(&store).expect("non-empty store renders");

        // Then the prompt lines appear in input order.
        let first = out.find("#first").expect("first present");
        let second = out.find("#second").expect("second present");
        let third = out.find("#third").expect("third present");
        assert!(first < second && second < third, "order wrong: {out:?}");
    }

    #[test]
    #[expect(clippy::expect_used, reason = "non-empty store is constructed above")]
    fn render_prompts_list_truncates_when_exceeding_soft_limit() {
        // Given a store whose rendered output would exceed Discord's
        // 2000-char limit — many prompts with long descriptions.
        let mut templates = Vec::new();
        for i in 0..200 {
            templates.push(template(
                &format!("prompt-{i:03}"),
                "lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do",
            ));
        }
        let store = PromptTemplateStore::from_vec(templates);

        // When rendering.
        let out = render_prompts_list(&store).expect("non-empty store renders");

        // Then the output stays under the hard limit.
        assert!(
            out.len() <= 2000,
            "rendered output exceeded Discord limit: {} bytes",
            out.len()
        );
        // And a truncation notice with a non-zero omitted count appears.
        assert!(
            out.contains("more not shown"),
            "missing truncation notice: {out:?}"
        );
        // And the very first prompt is still rendered (truncation is from
        // the tail, not the head).
        assert!(
            out.contains("`#prompt-000`"),
            "head prompt was truncated: {out:?}"
        );
    }
}
