//! Row building for the Bots sidebar section.
//!
//! The bot's *domain* — who its rows are, what their latest report
//! says, whether a report is stale — lives here in the owning slice. The
//! sidebar's section (`jinn-sidebar::sections::bots_section`) renders
//! these rows but never reconstructs them, which keeps the dependency one
//! way: sidebar → bot.
//!
//! Staleness: a report is stale when the parent session has produced history
//! since the report was published — that is the parent resuming work. The
//! comparison is per-bot, so one bot's fresh report never clears
//! a sibling's stale one.

use jinn_app_state::AppState;
use jinn_bot_msg::BotReport;

/// One bot row: the session id, its display name, and its latest report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotRow {
    /// The bot session's id.
    pub session_id: jinn_core_types::SessionId,
    /// The session name — the bot's identity in the UI.
    pub name: String,
    /// The most recent report body, if the bot has ever reported.
    pub latest_report: Option<String>,
    /// Whether the latest report predates the parent's latest activity.
    pub is_stale: bool,
    /// Whether the bot is still being composed, so nothing will run.
    /// Rendered beside the name, never inside it, so a rename can never
    /// reach the marker.
    pub is_prepping: bool,
    /// Whether the bot runs on its parent's completion. Its own
    /// marker, because "fires on its own" is a different fact from "cannot
    /// fire at all" and one glyph cannot say both.
    pub fires_on_parent_completion: bool,
}

/// The session whose bots the sidebar section is showing.
///
/// A bot is read in the context of the parent it reports to, so
/// viewing one resolves to that parent — which is also what makes an
/// bot's siblings visible from inside a sibling.
#[must_use]
pub fn bot_context_of(state: &AppState) -> jinn_core_types::SessionId {
    bot_context_of_split(&state.session)
}

/// Split-borrow variant of [`bot_context_of`].
#[must_use]
pub fn bot_context_of_split(
    session: &jinn_session_state::SessionMap,
) -> jinn_core_types::SessionId {
    let active = session.active_session();
    match active.parent_session() {
        Some(parent) => parent.clone(),
        None => active.session_id().clone(),
    }
}

/// Every bot in the active session's context, sorted by name, with
/// report and staleness data.
///
/// The section shows the bots *belonging to the session being read*:
/// sitting on a parent shows that parent's bots, and sitting on one of
/// its bots shows the whole set — the same list, because the bot
/// has no bots of its own and its siblings are the relevant context.
/// A section that listed every bot in the store would answer a
/// question the user did not ask from a screen they are not on.
#[must_use]
pub fn bot_rows(state: &AppState) -> Vec<BotRow> {
    bot_rows_split(&state.session, &state.frontend)
}

/// Split-borrow variant of [`bot_rows`].
///
/// Same rows in the same order — one implementation, so a caller holding
/// mutable frontend state (the sidebar's removal path) resolves the list
/// through exactly the function the render pass uses rather than rebuilding it.
#[must_use]
pub fn bot_rows_split(
    session: &jinn_session_state::SessionMap,
    _frontend: &jinn_kernel::state::frontend_state::FrontendState,
) -> Vec<BotRow> {
    let context = bot_context_of_split(session);
    let mut rows: Vec<BotRow> = session
        .iter()
        .filter(|(_, session)| {
            session.is_bot()
                && session.parent_session().as_ref() == Some(&context)
                && session.session_state() == jinn_session_store_msg::SessionState::Loaded
        })
        .map(|(id, bot)| {
            let latest = bot.latest_bot_report();
            let parent_activity = bot
                .parent_session()
                .as_ref()
                .and_then(|parent_id| session.get(parent_id))
                .map(|parent| *parent.last_history_activity_at());
            let is_stale = match (latest, parent_activity) {
                (Some(report), Some(activity)) => report.published_at < activity,
                _ => false,
            };
            BotRow {
                session_id: id.clone(),
                name: bot.title().unwrap_or("Untitled Session").to_owned(),
                latest_report: latest.map(|report: &BotReport| report.body.clone()),
                is_stale,
                is_prepping: bot.bot_is_prepping(),
                fires_on_parent_completion: bot.bot_fires_on_parent_completion(),
            }
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

/// The highlighted bot's full report log, for the history picker's
/// opener.
///
/// Returns `None` when nothing is highlighted or the selection is not a
/// loaded bot; a bot with no reports yields an empty vec, which
/// the picker renders as its never-reported state.
#[must_use]
pub fn highlighted_reports(state: &AppState) -> Option<Vec<jinn_bot_msg::BotReport>> {
    let id = state.frontend.with_sections(
        |sections: &jinn_sidebar_msg::SidebarSections| sections.bot.selected_id.clone(),
        || None,
    )?;
    state.session.get(&id).map(|a| a.bot_reports().to_vec())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    /// Builds state with `parent` active and `bots` installed under it.
    ///
    /// The section is scoped to the active session's context, so every
    /// fixture has to say which session the user is reading.
    fn state_viewing(parent: ChatSessionState, bots: Vec<ChatSessionState>) -> AppState {
        let mut state = AppState::default_with_scope_focus();
        let parent_id = parent.session_id().clone();
        state.session.insert(parent);
        for bot in bots {
            state.session.insert(bot);
        }
        state.session.set_active(parent_id);
        state
    }

    use super::*;
    use jinn_app_state::AppState;
    use jinn_session_state::ChatSessionState;

    #[rstest::rstest]
    #[test]
    fn a_report_goes_stale_when_the_parent_resumes() {
        // Given a bot whose report predates the parent's history.
        let mut parent = ChatSessionState::new();
        let mut bot = ChatSessionState::new_bot(&parent, true);
        // The report is published before the parent resumes.
        bot.append_bot_report("finding".to_owned());
        // The parent has since produced history (resumed work).
        parent.push_entry(jinn_core_types::chat_entry::ChatEntry::user(
            "parent resumed",
        ));
        let state = state_viewing(parent, vec![bot]);

        // When the section rows are built.
        let rows = bot_rows(&state);

        // Then the report renders stale.
        assert_eq!(rows.len(), 1);
        assert!(
            rows.first().expect("a row").is_stale,
            "a report older than the parent's resume is stale"
        );
    }

    #[rstest::rstest]
    #[test]
    fn a_fresh_report_is_not_stale() {
        // Given a bot whose report postdates the parent's history.
        let mut parent = ChatSessionState::new();
        let entry = jinn_core_types::chat_entry::ChatEntry::user("parent worked");
        parent.push_entry(entry);
        let bot = ChatSessionState::new_bot(&parent, true);
        let state = state_viewing(parent, vec![bot]);

        // When the section rows are built.
        let rows = bot_rows(&state);

        // Then the report is fresh — it was published after the resume.
        assert_eq!(rows.len(), 1);
        assert!(!rows.first().expect("a row").is_stale);
    }

    #[rstest::rstest]
    #[test]
    fn a_sibling_fresh_report_never_clears_another_bots_stale_one() {
        // Given two bots of the same parent: one reported before the
        // parent resumed, one after.
        let mut parent = ChatSessionState::new();
        parent.push_entry(jinn_core_types::chat_entry::ChatEntry::user("first work"));
        let parent_id = parent.session_id().clone();

        let mut early = ChatSessionState::new_bot(&parent, true);
        early.set_parent_session(parent_id.clone());
        early.set_title("early".to_owned());
        early.append_bot_report("early finding".to_owned());

        let mut late = ChatSessionState::new_bot(&parent, true);
        late.set_parent_session(parent_id.clone());
        late.set_title("late".to_owned());

        // The parent resumes work after the early report.
        parent.push_entry(jinn_core_types::chat_entry::ChatEntry::user(
            "parent resumed",
        ));

        // The late bot reports after the resume.
        late.append_bot_report("late finding".to_owned());

        let state = state_viewing(parent, vec![early, late]);

        // When the section rows are built.
        let rows = bot_rows(&state);

        // Then each bot's staleness is its own: the early report is
        // stale, the late one fresh — a sibling's report never launders it.
        let early_row = rows.iter().find(|r| r.name == "early").expect("early row");
        let late_row = rows.iter().find(|r| r.name == "late").expect("late row");
        assert!(early_row.is_stale);
        assert!(!late_row.is_stale);
    }

    #[rstest::rstest]
    #[test]
    fn renaming_an_bot_updates_its_row_name() {
        // Given a bot with a name.
        let parent = ChatSessionState::new();
        let mut bot = ChatSessionState::new_bot(&parent, true);
        bot.set_title("before".to_owned());
        let id = bot.session_id().clone();
        let mut state = state_viewing(parent, vec![bot]);

        // When the bot is renamed.
        state
            .session
            .get_mut(&id)
            .expect("the bot")
            .set_title("after".to_owned());

        // Then the section row carries the new name on the next build.
        let rows = bot_rows(&state);
        assert_eq!(rows.first().expect("a row").name, "after");
    }

    #[rstest::rstest]
    #[test]
    fn an_bot_that_never_reported_has_no_latest_report() {
        // Given a bot with no reports.
        let parent = ChatSessionState::new();
        let state = state_viewing(
            parent.clone(),
            vec![ChatSessionState::new_bot(&parent, true)],
        );

        // When the section rows are built.
        let rows = bot_rows(&state);

        // Then the row has no latest report — the renderer shows the
        // never-reported marker, and it is not stale (nothing to outdate).
        assert_eq!(rows.len(), 1);
        assert_eq!(rows.first().expect("a row").latest_report, None);
        assert!(!rows.first().expect("a row").is_stale);
    }

    #[rstest::rstest]
    #[test]
    fn a_triggered_bot_with_no_report_is_distinct_from_no_bot() {
        // Given a parent with one bot that has a trigger but no
        // report, and no other bots.
        let parent = ChatSessionState::new();
        let mut bot = ChatSessionState::new_bot(&parent, true);
        bot.set_bot_trigger(jinn_bot_msg::BotTrigger::ParentCompleted);
        let state = state_viewing(parent, vec![bot]);

        // When the section rows are built.
        let rows = bot_rows(&state);

        // Then the section has content — one row, never-reported — which is
        // distinct from a parent with no bots at all (zero rows, the
        // section collapses).
        assert_eq!(rows.len(), 1);
        assert_eq!(rows.first().expect("a row").latest_report, None);

        // And an otherwise identical state without the bot has none.
        let mut empty = AppState::default_with_scope_focus();
        empty.session.insert(ChatSessionState::new());
        assert_eq!(bot_rows(&empty), [] as [BotRow; 0]);
    }
}
