//! Creating a bot from a saved entry.
//!
//! The created bot takes its *configured* fields from the entry and
//! inherits the rest of its environment from the session it is created
//! under — the same environment `N` hands a fresh bot, and the same
//! persistence the fresh path performs. Nothing here reaches into the
//! sidebar or the session store: the commands it emits are the ordinary
//! ones a bot creation publishes.

use jinn_bot_msg::{BotModelSetting, BotTrigger};
use jinn_core_types::ChatEntry;
use jinn_preferences_config::schemas::BotEntryConfig;
use jinn_session_lifecycle_msg::event::SessionCreated;
use jinn_session_state::ChatSessionState;
use jinn_session_store_msg::PersistSession;
use jinn_slices::RouteResult as IntentResult;

use crate::saved_entry;

/// The line the created bot opens with, naming what it is.
const CREATED_NOTICE: &str =
    "🤖 Bot restored from a saved entry — its saved pins and configuration are in place.";

/// Builds the bot `entry` describes, as a child of `parent`.
///
/// Every field the entry carries is applied verbatim; every field it does
/// not carry is whatever `new_bot` inherited from the parent, which
/// is the whole point of "config first, environment inherited": an entry
/// saved on one machine must still run where the user's cwd and project
/// are.
///
/// The behavior, trigger, and prep mode come across unchanged, so a live
/// bot starts live rather than dropping back into seed mode.
#[must_use]
pub fn build(entry: &BotEntryConfig, parent: &ChatSessionState) -> ChatSessionState {
    let mut bot = ChatSessionState::new_bot(parent, true);
    apply_entry(&mut bot, entry);
    bot
}

/// Builds the bot `entry` describes, with no parent.
///
/// The same configuration [`build`] applies, onto a
/// [`ChatSessionState::new_standalone_bot`] base: the entry's environment
/// fields (cwd, project, MCP) still come from the source session, but the
/// created bot references no parent and its trigger is forced to
/// `manual` — every other trigger reads the parent's turn, and a
/// standalone bot has no parent to read.
#[must_use]
pub fn build_standalone(entry: &BotEntryConfig, source: &ChatSessionState) -> ChatSessionState {
    let mut bot = ChatSessionState::new_standalone_bot(source, true);
    apply_entry(&mut bot, entry);
    bot.set_bot_trigger(BotTrigger::Manual);
    bot
}

/// Applies an entry's configured fields onto a freshly built bot.
///
/// The shared half of [`build`] and [`build_standalone`]: both creation
/// shapes are "entry config over a constructor base", and letting the two
/// apply loops drift would make a saved bot restore differently depending
/// on which shape created it.
fn apply_entry(bot: &mut ChatSessionState, entry: &BotEntryConfig) {
    {
        let profile = bot.profile_mut();
        if let Some(model) = entry.configured_model() {
            profile.model = model.clone();
        }
        if let Some(persona) = &entry.persona_name {
            profile.persona_name.clone_from(persona);
        }
        // An absent filter leaves the source session's in place, so an
        // bot saved with no filter configured still gets whatever its
        // source had. A present filter replaces it wholesale — including an
        // allow-mode one, which is the whole point of being able to save
        // one, and including one naming nothing, which is a bot with
        // no tools at all.
        if let Some(filter) = &entry.tool_filter {
            profile.tool_filter = Some(filter.clone());
        }
        if let Some(filter) = &entry.skill_filter {
            profile.skill_filter = Some(filter.clone());
        }
        if let Some(effort) = entry.reasoning_effort {
            profile.reasoning_effort = Some(effort);
        }
    }
    bot.set_bot_behavior(entry.behavior);
    bot.set_bot_trigger(entry.trigger);
    bot.set_bot_is_prepping(entry.prep_mode);
    // The entry's `model` key is the bot's claim to its model, so the
    // created bot records the same claim. Without this an entry saved
    // with a model would read as inheriting on the next save and lose it.
    bot.set_bot_model_setting(BotModelSetting::of_configured_entry(
        entry.configured_model().is_some(),
    ));
    bot.set_seed_template(entry.seed_template.clone());
    // The entry's name is the bot's identity, both in the picker and
    // in the sessions list.
    bot.set_title(entry.name.clone());
    saved_entry::restore_pins(bot, &entry.pins);
}

/// The line the parent session is left with, naming what was attached to it.
///
/// Attaching is a load-up gesture: the entry was already saved, and the
/// conversation the user is on is none of the new bot's business. The
/// parent's log is therefore where the new bot is reported, rather
/// than the user being moved into it.
fn attached_notice(name: &str) -> String {
    format!("🤖 Bot \"{name}\" attached to this session.")
}

/// The messages a creation publishes, in the order they must arrive.
///
/// The parent is persisted first: a bot's own row names the parent,
/// and a parent that has never been written is a bot pointing at a
/// session the store has never heard of.
///
/// The log entries shape the sequence — the new bot is told what it
/// is (its notice), and the session it was created under is told what it
/// gained.
#[must_use]
pub fn creation_messages(
    parent_id: &jinn_core_types::SessionId,
    bot: &ChatSessionState,
) -> IntentResult {
    let bot_id = bot.session_id().clone();
    let cwd = bot.cwd().clone();
    IntentResult::empty()
        .with_message(PersistSession {
            session_id: parent_id.clone(),
        })
        .with_message(PersistSession {
            session_id: bot_id.clone(),
        })
        .with_message(SessionCreated {
            session_id: bot_id.clone(),
            cwd,
        })
        .with_message(jinn_session_history_msg::PushChatEntry {
            session_id: bot_id,
            entry: ChatEntry::system(CREATED_NOTICE),
            pin: None,
        })
        .with_message(jinn_session_history_msg::PushChatEntry {
            session_id: parent_id.clone(),
            entry: ChatEntry::system(attached_notice(bot.title().unwrap_or_default())),
            pin: None,
        })
}

/// Creates `entry` on `state` under the active session, publishing the
/// creation's messages.
///
/// The active session is left alone: attaching is a load-up gesture, and
/// the session the picker was opened from is none of the new bot's
/// business. Nothing here touches the focus stack either — the caller
/// decides what the user is looking at afterwards.
///
/// Returns `None` when the active session is gone or is itself an
/// bot: a bot of a bot is not a thing this feature
/// creates, and silently doing it would produce a lineage the trigger
/// actor has no rules for.
pub fn create_in_state(
    state: &mut jinn_app_state::AppState,
    entry: &BotEntryConfig,
) -> Option<IntentResult> {
    let parent_id = state.session.active_session_id().clone();
    let parent = state.session.get(&parent_id)?.clone();
    if parent.is_bot() {
        return None;
    }

    let bot = build(entry, &parent);
    let bot_id = bot.session_id().clone();
    state.session.insert(bot);
    // The new bot is created, not typed into: it is worth keeping
    // before it has said anything, exactly as a fresh `N` bot is.
    if let Some(a) = state.session.get_mut(&bot_id) {
        a.mark_interacted();
    }
    if let Some(p) = state.session.get_mut(&parent_id) {
        p.mark_interacted();
    }
    Some(creation_messages(&parent_id, state.session.get(&bot_id)?))
}

/// Creates `entry` on `state` as a standalone bot, switches to it, and
/// publishes the creation's messages.
///
/// The active session is the *environment source* only: it supplies cwd,
/// project, and profile the way an attached creation's parent does, but
/// nothing is recorded back onto it — no parent persist, no attached
/// notice, and no is-bot refusal, because there is no lineage being
/// formed that a bot-of-a-bot would confuse.
///
/// Unlike an attached creation, the user lands on the bot they spawned:
/// a standalone spawn is "start this program", not "load this in the
/// background", so the created session becomes active before the picker's
/// close signal pops it. The source session keeps the focus, so the
/// overlay/scope work is the caller's — this only moves the session.
///
/// The created bot itself is persisted and marked, exactly as an attached
/// one is.
pub fn create_standalone_in_state(
    state: &mut jinn_app_state::AppState,
    entry: &BotEntryConfig,
) -> Option<IntentResult> {
    let source_id = state.session.active_session_id().clone();
    let source = state.session.get(&source_id)?.clone();
    let bot = build_standalone(entry, &source);
    let bot_id = bot.session_id().clone();
    let cwd = bot.cwd().clone();
    state.session.insert(bot);
    if let Some(a) = state.session.get_mut(&bot_id) {
        a.mark_interacted();
    }
    state.session.set_active(bot_id.clone());
    Some(
        IntentResult::empty()
            .with_message(PersistSession {
                session_id: bot_id.clone(),
            })
            .with_message(SessionCreated {
                session_id: bot_id.clone(),
                cwd,
            })
            .with_message(jinn_session_history_msg::PushChatEntry {
                session_id: bot_id,
                entry: ChatEntry::system(CREATED_NOTICE),
                pin: None,
            }),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use jinn_bot_msg::BotModelSetting;
    use jinn_core_types::{FilterMode, ModelSelection, NameFilter};
    use jinn_preferences_config::schemas::BotEntryConfig;
    use jinn_session_state::ChatSessionState;

    use super::{build, build_standalone};

    /// The mode of the bot's tool filter, which must be present for
    /// every test here that asserts on it.
    fn filter_mode(bot: &ChatSessionState) -> FilterMode {
        bot.tool_filter().expect("bot carries a tool filter").mode
    }

    /// An entry carrying the given filters and nothing else.
    fn entry_with(
        tool_filter: Option<NameFilter>,
        skill_filter: Option<NameFilter>,
    ) -> BotEntryConfig {
        BotEntryConfig {
            name: "nightly".to_owned(),
            tool_filter,
            skill_filter,
            ..BotEntryConfig::default()
        }
    }

    #[rstest::rstest]
    fn a_created_bot_carries_the_saved_tool_filter() {
        // Given a parent withholding two tools, and an entry that saved so.
        let mut parent = ChatSessionState::new();
        parent.set_tool_filter(Some(NameFilter::deny([
            "bash".to_owned(),
            "write".to_owned(),
        ])));

        // When a bot is created from it.
        let bot = build(
            &entry_with(
                Some(NameFilter::deny(["bash".to_owned(), "write".to_owned()])),
                None,
            ),
            &parent,
        );

        // Then the created bot withholds the same tools, so a save and
        // restore is a no-op on its access rather than a widening of it.
        assert!(!bot.is_tool_enabled("bash"));
        assert!(!bot.is_tool_enabled("write"));
        assert!(bot.is_tool_enabled("read"));
    }

    #[rstest::rstest]
    fn a_created_bot_carries_a_saved_allow_filter_intact() {
        // Given an entry restricted to two tools by an allow filter.
        let parent = ChatSessionState::new();
        let filter = NameFilter {
            mode: FilterMode::Allow,
            names: ["read".to_owned(), "mcp__github__*".to_owned()]
                .into_iter()
                .collect(),
        };

        // When a bot is created from it.
        let bot = build(&entry_with(Some(filter.clone()), None), &parent);

        // Then the mode survives. Restoring it as a deny filter would invert
        // the bot's tool access — it would be left able to run
        // everything except the two tools it was meant to be limited to.
        assert_eq!(filter_mode(&bot), FilterMode::Allow);
        assert!(bot.is_tool_enabled("mcp__github__create_pr"));
        assert!(!bot.is_tool_enabled("bash"));
    }

    #[rstest::rstest]
    fn a_created_bot_inherits_the_parents_filter_when_none_was_saved() {
        // Given a parent withholding a tool and an entry that saved no filter.
        let mut parent = ChatSessionState::new();
        parent.set_tool_filter(Some(NameFilter::deny(["bash".to_owned()])));

        // When a bot is created from it.
        let bot = build(&entry_with(None, None), &parent);

        // Then it inherits. An absent filter means "as configured here",
        // which is what a bot saved with nothing overrides did before
        // filters existed.
        assert!(!bot.is_tool_enabled("bash"));
    }

    #[rstest::rstest]
    fn a_created_bot_inherits_the_parents_allow_filter_when_none_was_saved() {
        // Given a parent restricted by an allow filter, and an entry with none.
        let mut parent = ChatSessionState::new();
        parent.set_tool_filter(Some(NameFilter {
            mode: FilterMode::Allow,
            names: ["read".to_owned()].into_iter().collect(),
        }));

        // When a bot is created from it.
        let bot = build(&entry_with(None, None), &parent);

        // Then the restriction is inherited whole, mode included — inheriting
        // only the withheld names would leave the child permitted everything.
        assert_eq!(filter_mode(&bot), FilterMode::Allow);
        assert!(!bot.is_tool_enabled("bash"));
    }

    #[rstest::rstest]
    fn a_created_bot_carries_the_saved_skill_filter() {
        // Given a parent withholding a skill, and an entry that saved so.
        let mut parent = ChatSessionState::new();
        parent.set_skill_filter(Some(NameFilter::deny(["scream".to_owned()])));

        // When a bot is created from it.
        let bot = build(
            &entry_with(None, Some(NameFilter::deny(["dataviz".to_owned()]))),
            &parent,
        );

        // Then the saved skill filter replaces the parent's rather than
        // composing with it.
        assert!(!bot.is_skill_enabled("dataviz"));
        assert!(
            bot.is_skill_enabled("scream"),
            "the saved filter replaces the parent's, it does not merge with it"
        );
    }

    #[rstest::rstest]
    fn a_created_bot_inherits_the_parents_untouched_fields() {
        // Given a parent on a specific model.
        let mut parent = ChatSessionState::new();
        parent.set_model(ModelSelection::Single("zai/glm-4.7".to_owned()));

        // When a bot is created from an entry that configures no model.
        let bot = build(&entry_with(None, None), &parent);

        // Then it runs on the parent's model, as an entry without one always
        // did — the filter work must not have changed the inheritance path.
        assert_eq!(
            bot.model_selection(),
            &ModelSelection::Single("zai/glm-4.7".to_owned())
        );
    }

    #[rstest::rstest]
    fn an_bot_created_from_an_entry_without_a_model_inherits_its_model() {
        // Given an entry that names no model.
        let entry = entry_with(None, None);

        // When a bot is created from it.
        let bot = build(&entry, &ChatSessionState::new());

        // Then it records that its model is inherited. The model itself is
        // whatever it was created with; the setting is the bot's claim
        // to it, and an entry that made no claim must not produce one.
        assert_eq!(bot.bot_model_setting(), BotModelSetting::Inherit);
    }

    #[rstest::rstest]
    fn an_bot_created_from_an_entry_with_a_model_owns_it() {
        // Given an entry that names a model.
        let entry = BotEntryConfig {
            model: Some(ModelSelection::Single("zai/glm-4.7".to_owned())),
            ..entry_with(None, None)
        };

        // When a bot is created from it.
        let bot = build(&entry, &ChatSessionState::new());

        // Then it records that its model is its own. Without this the very
        // next save of that bot would drop the key the entry was
        // created from.
        assert_eq!(bot.bot_model_setting(), BotModelSetting::Fixed);
    }

    #[rstest::rstest]
    fn a_fresh_bot_owns_no_model_of_its_own() {
        // Given the parent a fresh `N` bot is created under.

        // When it is created.
        let parent = ChatSessionState::new();
        let bot = ChatSessionState::new_bot(&parent, true);

        // Then it inherits. A bot nobody created from a saved entry
        // was nobody's `model` key, so it starts the default way.
        assert_eq!(bot.bot_model_setting(), BotModelSetting::Inherit);
    }

    #[rstest::rstest]
    fn a_standalone_built_bot_has_no_parent_and_a_manual_trigger() {
        // Given a saved entry that asks for the parent-completed trigger
        // and a source session to take the environment from.
        let entry = BotEntryConfig {
            name: "lonely".to_owned(),
            trigger: jinn_bot_msg::BotTrigger::ParentCompleted,
            ..BotEntryConfig::default()
        };
        let source = ChatSessionState::new();

        // When the standalone bot is built from it.
        let bot = build_standalone(&entry, &source);

        // Then no parent link exists — the entry's trigger had nothing to
        // attach to, and a bot that fired on a parent it does not have
        // could never run at all.
        assert!(bot.parent_session().is_none());
        // And the trigger is forced back to manual: the save path and the
        // properties popup enforce the same rule for a parentless bot.
        assert_eq!(bot.bot_trigger(), jinn_bot_msg::BotTrigger::Manual);
    }

    #[rstest::rstest]
    fn a_standalone_built_bot_still_carries_the_entry_configuration() {
        // Given an entry with a tool filter, applied to a standalone base.
        let filter = NameFilter {
            mode: FilterMode::Allow,
            names: ["read".to_owned()].into_iter().collect(),
        };
        let entry = entry_with(Some(filter.clone()), None);
        let source = ChatSessionState::new();

        // When the standalone bot is built from it.
        let bot = build_standalone(&entry, &source);

        // Then the configuration survived the standalone path — a saved
        // bot spawned standalone is the same bot, minus the lineage.
        assert_eq!(bot.tool_filter(), Some(&filter));
        // And it took its cwd from the source like an attached bot would.
        assert_eq!(bot.cwd(), source.cwd());
    }
}
