//! Writing an attendant's properties back to `jinn.toml`.
//!
//! Saving is a read-modify-write against a hand-edited config file, so it is
//! deliberately two-phase. A new name writes on the first press; an existing
//! name arms on the first press and overwrites on the second. The entry under
//! that name is an attendant the user may have spent an afternoon building,
//! and a single stray key should not be able to replace it — so overwriting is
//! confirmed, not inferred.
//!
//! Every outcome, including every refusal, is reported on the popup's status
//! line. A key that does nothing looks like a broken key, and this popup is
//! the surface the user is looking at when they press one.
//!
//! `commit_pending_to_session` stays beside the write rather than beside the
//! form because the two are one operation: a save is not complete until the
//! live session has been brought in line with what was written, and splitting
//! them would leave a way to write the file without applying it.

use jinn_attendant_msg::{PopupStatus, SetField};
use jinn_core_types::{FilterMode, NameFilter};
use jinn_slices::RouteResult as IntentResult;
use jinn_slices::route::ActionCtx;

use super::properties_overlay::{AttendantPropertiesCell, app};

/// The `<c-s>` action: saves the popup's attendant to `jinn.toml`.
///
/// A new name saves on the first press. An existing name arms on the first
/// press and overwrites on the second, because the entry under that name
/// holds an attendant the user may have spent an afternoon building, and a
/// single stray key should not be able to replace it.
///
/// A session with no title cannot be saved: the title *is* the entry's
/// identity, and a session that never received a submission has none.
///
/// Every outcome is reported on the popup's status line, including the
/// refusals. A key that does nothing looks like a broken key, and the
/// popup is the surface the user is looking at when they press one.
pub(super) fn save_attendant(
    ctx: &mut ActionCtx<'_>,
    cell: &AttendantPropertiesCell,
) -> IntentResult {
    let popup = cell.read().clone();
    let Some(attendant_id) = popup.session_id.clone() else {
        return IntentResult::empty();
    };
    // Read the config before the state borrow: `app` takes `ctx` mutably.
    let config = ctx.config.clone();
    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    let Some(session) = state.session.get(&attendant_id) else {
        return IntentResult::empty();
    };
    let Some(name) = session
        .title()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
    else {
        return refuse_save(attendant_id, cell);
    };

    // The layer reads the live document, so a name collision is a property
    // of what is on disk rather than of anything this popup cached.
    // A malformed list is not an empty list: saving on top of it would
    // replace the user's entries with just this one, and the collision
    // check would have seen nothing. Read failures refuse the save.
    let mut entries =
        match config.get_list::<jinn_preferences_config::schemas::AttendantEntryConfig>() {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(err = ?error, "failed to read the saved attendants");
                cell.update(|p| {
                    p.report(PopupStatus::SaveFailed {
                        reason: "Cannot save: jinn.toml's saved attendants could not be read."
                            .to_owned(),
                    });
                });
                return IntentResult::empty();
            }
        };
    let collides = entries.iter().any(|existing| existing.name == name);
    if collides && !popup.save_armed {
        let armed = name.clone();
        cell.update(|p| {
            p.arm_save();
            p.report(PopupStatus::OverwriteArmed { name: armed });
        });
        return IntentResult::empty();
    }

    // Commit the popup's pending values to the session before reading it
    // back. Pressing save means wanting these settings, so the entry is
    // built from what the user is looking at — not from the pre-edit
    // session, which would write the old values and report success.
    //
    // `commit_pending_to_session` is the same path `<enter>` takes, so the
    // two ways of committing cannot drift apart.
    commit_pending_to_session(ctx, &popup, &attendant_id);

    let Some(state) = app(ctx) else {
        return IntentResult::empty();
    };
    let Some(session) = state.session.get(&attendant_id) else {
        return IntentResult::empty();
    };
    let entry = crate::saved_entry::entry_for_session(name.clone(), session);
    entries.retain(|existing| existing.name != entry.name);
    entries.push(entry);
    if let Err(error) =
        config.put_list::<jinn_preferences_config::schemas::AttendantEntryConfig>(&entries)
    {
        tracing::warn!(err = ?error, "failed to save the attendant to jinn.toml");
        let reason = format!("Could not save “{name}” — {}", describe_write_error(&error));
        cell.update(|p| p.report(PopupStatus::SaveFailed { reason }));
        return IntentResult::empty();
    }

    // Committed: the arm has served its purpose, and the popup's restore
    // point moves to what was just written so `<esc>` does not undo the
    // save it just reported as done.
    let saved = name;
    cell.update(|p| {
        p.disarm_save();
        p.commit_as_original();
        p.report(PopupStatus::Saved { name: saved });
    });

    // A save is a session edit, so it persists like any other. A freshly
    // created attendant was never interacted, and without this the write
    // below is silently dropped.
    IntentResult::empty().with_message(jinn_session_store_msg::PersistSession {
        session_id: attendant_id,
    })
}

/// Writes a properties popup's pending values onto its session.
///
/// Shared by `<enter>` (apply and close) and `<c-s>` (save and stay) so the
/// two commit paths cannot disagree about what "committed" means. Marks the
/// session interacted and touched, so a freshly created attendant persists.
///
/// The two set rows are written here rather than in the save path, because
/// `<enter>` alone has to leave the session holding what the panel showed.
/// A Live row writes nothing: the attendant inherits its parent's set, which
/// is what an unconfigured filter already means. The model row is written
/// unconditionally — it holds a value of its own rather than a reading of the
/// session, so there is no untouched case to leave alone.
pub(super) fn commit_pending_to_session(
    ctx: &mut ActionCtx<'_>,
    popup: &jinn_attendant_msg::AttendantPropertiesState,
    attendant_id: &jinn_core_types::SessionId,
) {
    let Some(state) = app(ctx) else {
        return;
    };
    let Some(session) = state.session.get_mut(attendant_id) else {
        return;
    };
    session.set_seed_template(popup.seed_template.input.clone());
    session.set_attendant_behavior(popup.pending_behavior);
    session.set_attendant_is_prepping(popup.pending_prep_mode);
    session.set_attendant_trigger(popup.pending_trigger);
    session.set_attendant_model_setting(popup.pending_model_setting);
    for field in [SetField::Tool, SetField::Skill] {
        // Only a row the user actually moved may write. An untouched row
        // opens as a reading of whatever filter the attendant already
        // carried -- a hand-written blocklist included -- and committing
        // that reading back unchanged is what keeps opening and saving an
        // untouched panel a no-op on every field.
        if !popup.set_touched(field) {
            continue;
        }
        let filter = committed_filter(popup, field);
        match field {
            SetField::Tool => session.set_tool_filter(filter),
            SetField::Skill => session.set_skill_filter(filter),
        }
    }
    // A fresh attendant was never interacted; without this the persist is
    // silently dropped.
    session.mark_interacted();
    session.touch();
}

/// What a set row commits to the session's filter.
///
/// A Live row commits an *unconfigured* filter, and that is the whole
/// reason the commit cannot simply skip a Live row. The row's mode is
/// derived from the session's filter — `OriginalValues::mode_of` reads an
/// allow-mode filter as Frozen — so leaving a thawed attendant's filter in
/// place left the session refusing everything outside the old allow list
/// while the panel said the set was live, and the next open of the panel
/// read Frozen again. There was no way back to Live. Writing the
/// unconfigured filter is what actually releases the set, and it means the
/// attendant inherits its parent's from there.
///
/// A row is never cleared because the attendant was *hand-written* as
/// frozen, though: an untouched popup commits the same unconfigured filter,
/// which is already what such an attendant inherits, so the file is left
/// alone and nothing widens behind the user's back. The freeze only ever
/// came from this panel, so this releases exactly what this panel put there.
///
/// A Frozen row whose capture is empty also commits an unconfigured
/// filter, which is the same release as a thaw. An empty allow list is read
/// as no filter at all, so persisting one would mark the attendant frozen
/// in `jinn.toml` while it went on inheriting everything — the one outcome
/// the row exists to prevent. That is why the refusal lives here, at the
/// write, rather than at the choice: refusing the mode instead, as this
/// once did, made the row unselectable on any attendant that had
/// discovered nothing yet, which is the default state of a fresh one.
fn committed_filter(
    popup: &jinn_attendant_msg::AttendantPropertiesState,
    field: SetField,
) -> Option<NameFilter> {
    // A captured set commits as an allow list over exactly those names.
    // An empty capture is a capture: it is a set frozen to nothing, and
    // it commits as an allow list naming nothing, which withholds every
    // name. That is the whole point of the row — refusing the empty case
    // left a freshly created attendant (which has the parent's filters
    // but none of its discovered skills) with no way to say so.
    // Live: no filter at all, so the attendant inherits its parent's.
    popup.pending_set(field).map(|names| NameFilter {
        mode: FilterMode::Allow,
        names: names.clone(),
    })
}

/// The user-facing half of a config write failure.
///
/// The report's own chain carries the path and the cause for a log reader;
/// the popup has room for one clause, and "the file could not be written"
/// is the part a user can act on.
fn describe_write_error(error: &error_stack::Report<jinn_config::ConfigError>) -> String {
    match error.current_context() {
        jinn_config::ConfigError::Storage { .. } => "jinn.toml could not be written.".to_owned(),
        _ => "jinn.toml could not be updated.".to_owned(),
    }
}

/// Tells the attendant why it was not saved, in its own chat log, and says
/// the same thing on the popup's status line.
///
/// The popup targets a highlighted session that is not necessarily the
/// active one, so the chat line goes to the attendant itself — a refusal
/// written into some other session's log would be both invisible and
/// misleading. The status line is here because the popup covers the chat
/// log while it is open, and the user pressing `<c-s>` is looking at the
/// popup, not behind it.
fn refuse_save(
    attendant_id: jinn_core_types::SessionId,
    cell: &AttendantPropertiesCell,
) -> IntentResult {
    const REASON: &str = "This attendant has no name yet — send it a message first.";
    cell.update(|p| {
        p.report(PopupStatus::SaveFailed {
            reason: format!("Cannot save: {REASON}"),
        });
    });
    IntentResult::empty().with_message(jinn_session_history_msg::PushChatEntry {
        session_id: attendant_id,
        entry: jinn_core_types::ChatEntry::error(format!("Cannot save: {REASON}")),
        pin: None,
    })
}
